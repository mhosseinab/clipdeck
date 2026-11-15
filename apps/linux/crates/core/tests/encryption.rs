use rusqlite::Connection;
use std::path::PathBuf;

struct TestProfile(PathBuf);
impl TestProfile {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "clipdeck-encryption-{}",
            clipdeck_core::store::uuid_v4ish()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn database(&self) -> PathBuf {
        self.0.join("history.sqlite3")
    }
}
impl Drop for TestProfile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn encryption_backend_does_not_ignore_the_key() {
    let profile = TestProfile::new();
    let connection = Connection::open(profile.database()).unwrap();
    connection
        .pragma_update(None, "key", "test-only-passphrase")
        .unwrap();
    connection
        .execute_batch(
            "CREATE TABLE clips(content TEXT); INSERT INTO clips VALUES('private sample');",
        )
        .unwrap();
    drop(connection);
    let bytes = std::fs::read(profile.database()).unwrap();
    assert!(
        !bytes.starts_with(b"SQLite format 3\0"),
        "encryption pragma was silently ignored"
    );
    assert!(Connection::open(profile.database())
        .unwrap()
        .query_row("SELECT count(*) FROM sqlite_master", [], |row| row
            .get::<_, i64>(0))
        .is_err());
}

use clipdeck_core::crypto::{
    copy_image_transform, export_database, open_connection, read_image, write_image, EncryptionKey,
};
use rusqlite::{params, OpenFlags};

fn writer() -> OpenFlags {
    OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
}
fn reader() -> OpenFlags {
    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
}
fn key(value: &str) -> EncryptionKey {
    EncryptionKey::from_hex(&value.repeat(32)).unwrap()
}

#[test]
fn raw_keys_fail_closed_and_keep_page_memory_bounded() {
    let profile = TestProfile::new();
    let correct = key("a1");
    let connection = open_connection(&profile.database(), writer(), Some(&correct)).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE items(value TEXT); INSERT INTO items VALUES('private Unicode 🦀');",
        )
        .unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA cache_size", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        -2048
    );
    assert_eq!(
        connection
            .query_row("PRAGMA temp_store", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    drop(connection);
    assert!(open_connection(&profile.database(), reader(), None).is_err());
    assert!(open_connection(&profile.database(), reader(), Some(&key("b2"))).is_err());
    let reopened = open_connection(&profile.database(), reader(), Some(&correct)).unwrap();
    assert_eq!(
        reopened
            .query_row("SELECT value FROM items", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "private Unicode 🦀"
    );
}

#[test]
fn database_exports_preserve_source_and_all_payloads_in_both_modes() {
    let profile = TestProfile::new();
    let original = profile.database();
    let encrypted = profile.0.join("encrypted.sqlite3");
    let plaintext = profile.0.join("plain.sqlite3");
    let secret = key("c3");
    let connection = open_connection(&original, writer(), None).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE items(id TEXT PRIMARY KEY,body TEXT,bytes BLOB); PRAGMA user_version=7;",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO items VALUES(?1,?2,?3)",
            params![
                "item",
                "exact 🦀 text\0with NUL",
                b"binary payload\0\xff".as_slice()
            ],
        )
        .unwrap();
    drop(connection);
    let before = std::fs::read(&original).unwrap();
    export_database(&original, &encrypted, None, Some(&secret)).unwrap();
    assert_eq!(std::fs::read(&original).unwrap(), before);
    assert!(!std::fs::read(&encrypted)
        .unwrap()
        .starts_with(b"SQLite format 3\0"));
    export_database(&encrypted, &plaintext, Some(&secret), None).unwrap();
    let reopened = open_connection(&plaintext, reader(), None).unwrap();
    assert_eq!(
        reopened
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        7
    );
    let (body, bytes): (String, Vec<u8>) = reopened
        .query_row("SELECT body,bytes FROM items", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(body, "exact 🦀 text\0with NUL");
    assert_eq!(bytes, b"binary payload\0\xff");
}

#[test]
fn failed_export_never_replaces_existing_data() {
    let profile = TestProfile::new();
    let original = profile.database();
    let target = profile.0.join("occupied.sqlite3");
    let connection = open_connection(&original, writer(), None).unwrap();
    connection
        .execute_batch("CREATE TABLE items(body TEXT); INSERT INTO items VALUES('keep me');")
        .unwrap();
    drop(connection);
    let before = std::fs::read(&original).unwrap();
    std::fs::write(&target, b"another file").unwrap();
    assert!(export_database(&original, &target, None, Some(&key("d4"))).is_err());
    assert_eq!(std::fs::read(&original).unwrap(), before);
    assert_eq!(std::fs::read(target).unwrap(), b"another file");
    assert_eq!(std::fs::read_dir(&profile.0).unwrap().count(), 2);
}

#[test]
fn active_wal_reader_prevents_conversion_snapshot_switch() {
    let profile = TestProfile::new();
    let connection = open_connection(&profile.database(), writer(), None).unwrap();
    connection.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE items(value INTEGER); INSERT INTO items VALUES(1);").unwrap();
    let active = open_connection(&profile.database(), reader(), None).unwrap();
    active.execute_batch("BEGIN;").unwrap();
    assert_eq!(
        active
            .query_row("SELECT count(*) FROM items", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    connection
        .execute("INSERT INTO items VALUES(2)", [])
        .unwrap();
    let target = profile.0.join("blocked.sqlite3");
    assert!(export_database(&profile.database(), &target, None, Some(&key("e5"))).is_err());
    assert!(!target.exists());
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM items", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn encrypted_image_files_authenticate_key_name_and_contents() {
    let profile = TestProfile::new();
    let path = profile.0.join("clip.png");
    let secret = key("f6");
    let content = b"original binary image bytes\0\xff".to_vec();
    write_image(&path, content.clone(), Some(&secret)).unwrap();
    let first_ciphertext = std::fs::read(&path).unwrap();
    assert_ne!(first_ciphertext, content);
    write_image(&path, content.clone(), Some(&secret)).unwrap();
    assert_ne!(
        std::fs::read(&path).unwrap(),
        first_ciphertext,
        "each write needs a fresh nonce"
    );
    assert_eq!(read_image(&path, Some(&secret)).unwrap(), content);
    assert!(read_image(&path, None).is_err());
    assert!(read_image(&path, Some(&key("a7"))).is_err());
    let renamed = profile.0.join("other.png");
    std::fs::copy(&path, &renamed).unwrap();
    assert!(read_image(&renamed, Some(&secret)).is_err());
    let mut bytes = std::fs::read(&path).unwrap();
    let index = bytes.len() / 2;
    bytes[index] ^= 1;
    std::fs::write(&path, bytes).unwrap();
    assert!(read_image(&path, Some(&secret)).is_err());
}

#[test]
fn image_conversion_keeps_source_and_never_falls_back_to_plaintext() {
    let profile = TestProfile::new();
    let old = profile.0.join("old");
    let encrypted = profile.0.join("encrypted");
    let plain = profile.0.join("plain");
    for path in [&old, &encrypted, &plain] {
        std::fs::create_dir(path).unwrap()
    }
    let secret = key("b8");
    let bytes = b"sample original PNG content".to_vec();
    write_image(&old.join("clip.png"), bytes.clone(), None).unwrap();
    assert!(read_image(&old.join("clip.png"), Some(&secret)).is_err());
    copy_image_transform(
        &old.join("clip.png"),
        &encrypted.join("clip.png"),
        None,
        Some(&secret),
    )
    .unwrap();
    assert_eq!(std::fs::read(old.join("clip.png")).unwrap(), bytes);
    copy_image_transform(
        &encrypted.join("clip.png"),
        &plain.join("clip.png"),
        Some(&secret),
        None,
    )
    .unwrap();
    assert_eq!(std::fs::read(plain.join("clip.png")).unwrap(), bytes);
    assert!(copy_image_transform(
        &encrypted.join("clip.png"),
        &plain.join("bad.png"),
        Some(&key("ff")),
        None
    )
    .is_err());
    assert!(!plain.join("bad.png").exists());
}

#[test]
fn key_parser_rejects_invalid_material() {
    assert!(EncryptionKey::from_hex("short").is_err());
    assert!(EncryptionKey::from_hex(&"zz".repeat(32)).is_err());
    let secret = EncryptionKey::generate().unwrap();
    let hex = secret.expose_hex();
    let parsed = EncryptionKey::from_hex(&hex).unwrap();
    assert!(parsed.expose_hex().as_str() == hex.as_str());
}

#[test]
fn image_helpers_reject_symlinks_and_use_private_file_modes() {
    use std::os::unix::fs::PermissionsExt;
    let profile = TestProfile::new();
    let target = profile.0.join("target.png");
    let link = profile.0.join("link.png");
    write_image(&target, b"sample".to_vec(), Some(&key("ab"))).unwrap();
    assert_eq!(
        std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o600
    );
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(read_image(&link, Some(&key("ab"))).is_err());
    assert!(write_image(&link, b"replace".to_vec(), Some(&key("ab"))).is_err());
    assert_eq!(read_image(&target, Some(&key("ab"))).unwrap(), b"sample");
}
