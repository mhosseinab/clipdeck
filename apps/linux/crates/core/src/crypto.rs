//! Profile encryption primitives. Keys stay in memory; the desktop app owns
//! keyring access and switches complete storage generations after conversion.
use chacha20poly1305::{
    aead::{AeadInPlace, KeyInit},
    Tag, XChaCha20Poly1305, XNonce,
};
use hkdf::Hkdf;
use rusqlite::{params, Connection, OpenFlags};
use sha2::Sha256;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use zeroize::Zeroizing;

const IMAGE_MAGIC: &[u8] = b"CLIPDECKIMAGE\x01";
const NONCE_BYTES: usize = 24;
const TAG_BYTES: usize = 16;
const IMAGE_KEY_CONTEXT: &[u8] = b"io.github.mhosseinab.clipdeck/image-file/v1";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// An immutable profile key. Clones share the same zeroized-on-drop allocation.
/// Deliberately does not implement Debug or Serialize.
#[derive(Clone)]
pub struct EncryptionKey(Arc<Zeroizing<[u8; 32]>>);
impl EncryptionKey {
    pub fn generate() -> io::Result<Self> {
        let mut bytes = Zeroizing::new([0; 32]);
        File::open("/dev/urandom")?.read_exact(bytes.as_mut())?;
        Ok(Self(Arc::new(bytes)))
    }
    pub fn from_hex(value: &str) -> Result<Self, String> {
        if value.len() != 64 {
            return Err("The desktop keyring returned an invalid profile key".into());
        }
        let mut bytes = Zeroizing::new([0; 32]);
        for (output, pair) in bytes.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
            let nibble = |byte: u8| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                b'A'..=b'F' => Some(byte - b'A' + 10),
                _ => None,
            };
            let high =
                nibble(pair[0]).ok_or("The desktop keyring returned an invalid profile key")?;
            let low =
                nibble(pair[1]).ok_or("The desktop keyring returned an invalid profile key")?;
            *output = (high << 4) | low;
        }
        Ok(Self(Arc::new(bytes)))
    }
    pub fn expose_hex(&self) -> Zeroizing<String> {
        let mut result = Zeroizing::new(String::with_capacity(64));
        self.append_hex(&mut result);
        result
    }
    fn append_hex(&self, output: &mut String) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for byte in self.0.iter() {
            output.push(HEX[(byte >> 4) as usize] as char);
            output.push(HEX[(byte & 15) as usize] as char);
        }
    }
    fn sql_key(&self) -> Zeroizing<String> {
        let mut result = Zeroizing::new(String::with_capacity(67));
        result.push_str("x'");
        self.append_hex(&mut result);
        result.push('\'');
        result
    }
    fn image_key(&self) -> Zeroizing<[u8; 32]> {
        let mut result = Zeroizing::new([0; 32]);
        Hkdf::<Sha256>::new(None, self.0.as_ref().as_ref())
            .expand(IMAGE_KEY_CONTEXT, result.as_mut())
            .expect("32 bytes is a valid SHA256 HKDF output");
        result
    }
}

/// Apply a raw SQLCipher key before the first schema access. Missing/wrong keys
/// fail when authenticating the schema; there is no fresh-plaintext fallback.
pub fn open_connection(
    path: &Path,
    flags: OpenFlags,
    key: Option<&EncryptionKey>,
) -> rusqlite::Result<Connection> {
    if flags.contains(OpenFlags::SQLITE_OPEN_CREATE) && !path.exists() {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
        {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(error))),
        }
    }
    let connection = Connection::open_with_flags(path, flags | OpenFlags::SQLITE_OPEN_NOFOLLOW)?;
    if let Some(key) = key {
        let version: String =
            connection.query_row("PRAGMA cipher_version", [], |row| row.get(0))?;
        if version.is_empty() {
            return Err(rusqlite::Error::InvalidQuery);
        }
        // Redact key-setting failures: an unexpected SQL-input error may include
        // its statement, which must never expose raw key material to logs.
        connection
            .pragma_update(None, "key", key.sql_key().as_str())
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        connection.pragma_update(None, "cipher_memory_security", "ON")?;
    }
    connection.busy_timeout(Duration::from_millis(250))?;
    connection
        .execute_batch("PRAGMA cache_size=-2048; PRAGMA mmap_size=0; PRAGMA temp_store=MEMORY;")?;
    connection.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
        row.get::<_, i64>(0)
    })?;
    Ok(connection)
}

/// Export into a new generation without replacing the old database. The app
/// must own its profile lock and stop existing reader/writer connections first.
pub fn export_database(
    source: &Path,
    destination: &Path,
    old_key: Option<&EncryptionKey>,
    new_key: Option<&EncryptionKey>,
) -> Result<(), String> {
    export_database_inner(source, destination, old_key, new_key)
        .map_err(|error| format!("Could not convert clipboard database: {error}"))
}
fn export_database_inner(
    source: &Path,
    destination: &Path,
    old_key: Option<&EncryptionKey>,
    new_key: Option<&EncryptionKey>,
) -> Result<(), Box<dyn std::error::Error>> {
    let source_flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = open_connection(source, source_flags, old_key)?;
    let (busy, _, _): (i64, i64, i64) =
        connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
    if busy != 0 {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "Clipboard database is in use; close other database readers and retry",
        )
        .into());
    }
    connection.query_row("PRAGMA locking_mode=EXCLUSIVE", [], |r| {
        r.get::<_, String>(0)
    })?;
    connection.execute_batch("BEGIN EXCLUSIVE; COMMIT;")?;
    let version: i64 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let reserved = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(destination)?;
    drop(reserved);
    let mut cleanup = OwnedFile::new(destination.to_owned());
    let target = destination.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Database path is not valid UTF-8",
        )
    })?;
    let target_key = new_key
        .map(EncryptionKey::sql_key)
        .unwrap_or_else(|| Zeroizing::new(String::new()));
    connection
        .execute(
            "ATTACH DATABASE ?1 AS converted KEY ?2",
            params![target, target_key.as_str()],
        )
        .map_err(|_| io::Error::other("Could not attach the conversion database"))?;
    connection.query_row("PRAGMA converted.journal_mode=DELETE", [], |r| {
        r.get::<_, String>(0)
    })?;
    connection.execute_batch("PRAGMA converted.cache_size=-2048; PRAGMA converted.mmap_size=0;")?;
    connection.execute_batch("SELECT sqlcipher_export('converted');")?;
    connection.execute_batch(&format!("PRAGMA converted.user_version={version};"))?;
    let integrity: String =
        connection.query_row("PRAGMA converted.integrity_check", [], |r| r.get(0))?;
    if integrity != "ok" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Converted database failed its integrity check",
        )
        .into());
    }
    connection.execute_batch("DETACH DATABASE converted;")?;
    connection.close().map_err(|(_, error)| error)?;
    let check = open_connection(
        destination,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        new_key,
    )?;
    let integrity: String = check.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if integrity != "ok" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Converted database failed validation",
        )
        .into());
    }
    drop(check);
    File::open(destination)?.sync_all()?;
    sync_parent(destination)?;
    cleanup.keep = true;
    Ok(())
}

/// Write a single image atomically, encrypted when the profile has a key.
/// Encryption modifies the owned input in place rather than duplicating it.
pub fn write_image(path: &Path, bytes: Vec<u8>, key: Option<&EncryptionKey>) -> io::Result<()> {
    reject_symlink(path)?;
    let mut bytes = Zeroizing::new(bytes);
    let mut nonce = [0; NONCE_BYTES];
    let mut tag = None;
    if let Some(key) = key {
        File::open("/dev/urandom")?.read_exact(&mut nonce)?;
        let derived = key.image_key();
        let cipher = XChaCha20Poly1305::new_from_slice(derived.as_ref())
            .map_err(|_| io::Error::other("Invalid image encryption key"))?;
        tag = Some(
            cipher
                .encrypt_in_place_detached(
                    XNonce::from_slice(&nonce),
                    &image_aad(path, &nonce)?,
                    bytes.as_mut_slice(),
                )
                .map_err(|_| io::Error::other("Could not encrypt clipboard image"))?,
        );
    }
    let (temporary, mut file) = reserve_temporary(path)?;
    let mut cleanup = OwnedFile::new(temporary.clone());
    if key.is_some() {
        file.write_all(IMAGE_MAGIC)?;
        file.write_all(&nonce)?
    }
    file.write_all(&bytes)?;
    if let Some(tag) = tag {
        file.write_all(&tag)?
    }
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)?;
    sync_parent(path)?;
    cleanup.keep = true;
    Ok(())
}

pub fn read_image(path: &Path, key: Option<&EncryptionKey>) -> io::Result<Vec<u8>> {
    let mut file = open_regular_image(path)?;
    if let Some(key) = key {
        let mut magic = [0; 14];
        file.read_exact(&mut magic)?;
        if magic != IMAGE_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Clipboard image is not encrypted",
            ));
        }
        let mut nonce = [0; NONCE_BYTES];
        file.read_exact(&mut nonce)?;
        let mut bytes = Zeroizing::new(Vec::new());
        file.read_to_end(&mut bytes)?;
        if bytes.len() < TAG_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Encrypted image is truncated",
            ));
        }
        let tag_offset = bytes.len() - TAG_BYTES;
        let tag_bytes: [u8; TAG_BYTES] = bytes[tag_offset..].try_into().expect("fixed tag size");
        bytes.truncate(tag_offset);
        let derived = key.image_key();
        let cipher = XChaCha20Poly1305::new_from_slice(derived.as_ref())
            .map_err(|_| io::Error::other("Invalid image encryption key"))?;
        cipher
            .decrypt_in_place_detached(
                XNonce::from_slice(&nonce),
                &image_aad(path, &nonce)?,
                bytes.as_mut_slice(),
                Tag::from_slice(&tag_bytes),
            )
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Clipboard image authentication failed",
                )
            })?;
        Ok(std::mem::take(&mut *bytes))
    } else {
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        if bytes.starts_with(IMAGE_MAGIC) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Clipboard image requires the desktop keyring",
            ));
        }
        Ok(bytes)
    }
}

pub fn copy_image_transform(
    source: &Path,
    destination: &Path,
    old_key: Option<&EncryptionKey>,
    new_key: Option<&EncryptionKey>,
) -> io::Result<()> {
    let bytes = read_image(source, old_key)?;
    write_image(destination, bytes, new_key)
}

fn image_aad(path: &Path, nonce: &[u8; NONCE_BYTES]) -> io::Result<Vec<u8>> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Image has no filename"))?
        .as_encoded_bytes();
    let mut aad = Vec::with_capacity(IMAGE_MAGIC.len() + NONCE_BYTES + 8 + name.len());
    aad.extend_from_slice(IMAGE_MAGIC);
    aad.extend_from_slice(nonce);
    aad.extend_from_slice(&(name.len() as u64).to_le_bytes());
    aad.extend_from_slice(name);
    Ok(aad)
}
fn reject_symlink(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.is_file() => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Image must be a regular file, not a symlink",
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}
fn open_regular_image(path: &Path) -> io::Result<File> {
    let expected = fs::symlink_metadata(path)?;
    if !expected.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Image must be a regular file, not a symlink",
        ));
    }
    let file = File::open(path)?;
    let actual = file.metadata()?;
    if !actual.is_file() || expected.dev() != actual.dev() || expected.ino() != actual.ino() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Image changed while it was being opened",
        ));
    }
    Ok(file)
}
fn reserve_temporary(path: &Path) -> io::Result<(PathBuf, File)> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "Image has no parent directory")
    })?;
    for _ in 0..16 {
        let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(
            ".clipdeck-write-{}-{suffix}.tmp",
            std::process::id()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Could not reserve a temporary image file",
    ))
}
fn sync_parent(path: &Path) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "File has no parent directory")
    })?;
    File::open(parent)?.sync_all()
}
struct OwnedFile {
    path: PathBuf,
    keep: bool,
}
impl OwnedFile {
    fn new(path: PathBuf) -> Self {
        Self { path, keep: false }
    }
}
impl Drop for OwnedFile {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_file(&self.path);
            for suffix in ["-wal", "-shm", "-journal"] {
                let _ = fs::remove_file(format!("{}{suffix}", self.path.display()));
            }
        }
    }
}
