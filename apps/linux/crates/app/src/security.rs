//! Profile encryption policy and desktop-keyring access. Key material never
//! appears in the on-disk policy. Conversion runs only under the profile lock.
use crate::events::AppEvent;
use clipdeck_core::{
    crypto::{self, EncryptionKey},
    Store,
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};
use winit::event_loop::EventLoopProxy;

const POLICY_FILE: &str = "clipdeck-security.json";
const GENERATION_MARKER: &str = ".clipdeck-storage";
const KEYRING_STARTUP_ATTEMPTS: usize = 30;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivacyConfig {
    pub configured: bool,
    pub encrypted: bool,
    pub pending: Option<bool>,
    pub data_dir: String,
}
impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            configured: false,
            encrypted: false,
            pending: None,
            data_dir: "store".into(),
        }
    }
}
fn valid_directory(name: &str) -> bool {
    (name == "store" || name.starts_with("store-"))
        && name.len() <= 128
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && Path::new(name)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
pub fn load_config(profile: &Path) -> Result<PrivacyConfig, String> {
    let path = profile.join(POLICY_FILE);
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(PrivacyConfig::default()),
        Err(e) => return Err(e.to_string()),
        Ok(m) if !m.is_file() || m.file_type().is_symlink() => {
            return Err("Encryption policy must be a regular file".into())
        }
        _ => {}
    }
    let value: serde_json::Value =
        serde_json::from_reader(File::open(&path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("Invalid encryption policy: {e}"))?;
    let configured = value
        .get("configured")
        .and_then(|v| v.as_bool())
        .ok_or("Missing encryption choice")?;
    let encrypted = value
        .get("encrypted")
        .and_then(|v| v.as_bool())
        .ok_or("Missing encryption mode")?;
    let pending = match value.get("pending") {
        None | Some(serde_json::Value::Null) => None,
        Some(v) => Some(v.as_bool().ok_or("Invalid pending encryption mode")?),
    };
    let data_dir = value
        .get("data_dir")
        .and_then(|v| v.as_str())
        .ok_or("Missing storage directory")?
        .to_owned();
    if !valid_directory(&data_dir) || (encrypted && !configured) {
        return Err("Invalid encryption policy".into());
    }
    Ok(PrivacyConfig {
        configured,
        encrypted,
        pending,
        data_dir,
    })
}
fn save_config(profile: &Path, config: &PrivacyConfig) -> Result<(), String> {
    fs::create_dir_all(profile).map_err(|e| e.to_string())?;
    fs::set_permissions(profile, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    let parent = File::open(profile).map_err(|e| e.to_string())?;
    let tmp = profile.join(format!(
        ".security-{}.tmp",
        clipdeck_core::store::uuid_v4ish()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| e.to_string())?;
        serde_json::to_writer(&mut file,&serde_json::json!({"configured":config.configured,"encrypted":config.encrypted,"pending":config.pending,"data_dir":config.data_dir})).map_err(|e|e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        fs::rename(&tmp, profile.join(POLICY_FILE)).map_err(|e| e.to_string())?;
        parent.sync_all().map_err(|e| e.to_string())
    })();
    let _ = fs::remove_file(tmp);
    result
}
trait Keyring {
    fn get(&self, profile: &Path) -> Result<Option<EncryptionKey>, String>;
    fn put(&self, profile: &Path, key: &EncryptionKey) -> Result<(), String>;
}
struct DesktopKeyring;
fn account(profile: &Path) -> Result<String, String> {
    let path = profile.canonicalize().map_err(|e| e.to_string())?;
    Ok(format!(
        "{:x}",
        Sha256::digest(path.as_os_str().as_encoded_bytes())
    ))
}
impl Keyring for DesktopKeyring {
    fn get(&self, profile: &Path) -> Result<Option<EncryptionKey>, String> {
        let mut output = Command::new("secret-tool")
            .args([
                "lookup",
                "application",
                crate::APP_ID,
                "profile",
                &account(profile)?,
            ])
            .output()
            .map_err(|e| format!("Desktop keyring is unavailable: {e}"))?;
        if !output.status.success() {
            if output.status.code() == Some(1) && output.stderr.is_empty() {
                return Ok(None);
            }
            return Err(format!(
                "Cannot unlock desktop keyring: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let result = std::str::from_utf8(&output.stdout)
            .map_err(|_| "Invalid keyring entry".to_string())
            .and_then(|s| EncryptionKey::from_hex(s.trim()))
            .map(Some);
        output.stdout.fill(0);
        result
    }
    fn put(&self, profile: &Path, key: &EncryptionKey) -> Result<(), String> {
        let mut child = Command::new("secret-tool")
            .args([
                "store",
                "--label=ClipDeck encrypted history",
                "application",
                crate::APP_ID,
                "profile",
                &account(profile)?,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Desktop keyring is unavailable: {e}"))?;
        let hex = key.expose_hex();
        child
            .stdin
            .take()
            .ok_or("Cannot send key to desktop keyring")?
            .write_all(hex.as_bytes())
            .map_err(|e| e.to_string())?;
        let output = child.wait_with_output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "Cannot save key in desktop keyring: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(())
    }
}
fn required_key(provider: &impl Keyring, profile: &Path) -> Result<EncryptionKey, String> {
    provider
        .get(profile)?
        .ok_or_else(|| "Encrypted history is locked: its desktop-keyring key is missing".into())
}
pub fn choose_plaintext(profile: &Path) -> Result<PrivacyConfig, String> {
    let mut config = load_config(profile)?;
    if config.encrypted || config.pending.is_some() {
        return Err("An encryption change requires a restart".into());
    }
    config.configured = true;
    save_config(profile, &config)?;
    Ok(config)
}
fn prepare_with(profile: &Path, enabled: bool, provider: &impl Keyring) -> Result<(), String> {
    let mut config = load_config(profile)?;
    if config.encrypted {
        let _ = required_key(provider, profile)?;
    }
    if enabled && provider.get(profile)?.is_none() {
        let key = EncryptionKey::generate().map_err(|e| e.to_string())?;
        provider.put(profile, &key)?;
        let _ = required_key(provider, profile)?;
    }
    config.pending = Some(enabled);
    save_config(profile, &config)
}
pub fn prepare_change(profile: PathBuf, enabled: bool, events: EventLoopProxy<AppEvent>) {
    std::thread::spawn(move || {
        let result = prepare_with(&profile, enabled, &DesktopKeyring);
        let _ = events.send_event(AppEvent::EncryptionPrepared { enabled, result });
    });
}
pub fn restart(profile: &Path) -> Result<(), String> {
    Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
        .arg("run")
        .env("CLIPDECK_USER_DATA", profile)
        .env("CLIPDECK_ENCRYPTION_RESTART", "1")
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| format!("Cannot restart ClipDeck: {e}"))?;
    Ok(())
}
fn real_directory(path: &Path, create: bool) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(format!(
                "Storage must be a real directory: {}",
                path.display()
            ))
        }
        Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|e| e.to_string())?;
        }
        Err(error) => {
            return Err(format!(
                "Storage directory unavailable at {}: {error}",
                path.display()
            ))
        }
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())
}
fn validate_generation(path: &Path) -> Result<(), String> {
    real_directory(path, false)?;
    real_directory(&path.join("images"), false)?;
    let db = fs::symlink_metadata(path.join("clipdeck.sqlite3"))
        .map_err(|e| format!("History database is missing: {e}"))?;
    if !db.is_file() || db.file_type().is_symlink() || db.len() == 0 {
        return Err(
            "History database is missing or invalid; refusing to create an empty replacement"
                .into(),
        );
    }
    Ok(())
}
fn mark_generation(path: &Path) -> Result<(), String> {
    real_directory(path, true)?;
    real_directory(&path.join("images"), true)?;
    let marker = path.join(GENERATION_MARKER);
    match fs::symlink_metadata(&marker) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {}
        Ok(_) => return Err("Invalid storage-generation marker".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(marker)
                .map_err(|e| e.to_string())?
                .write_all(b"ClipDeck storage generation 1\n")
                .map_err(|e| e.to_string())?;
        }
        Err(error) => return Err(error.to_string()),
    }
    Ok(())
}
fn remove_private_path(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {
            fs::remove_dir_all(path).map_err(|e| e.to_string())
        }
        Ok(_) => fs::remove_file(path).map_err(|e| e.to_string()),
    }
}
fn cleanup_previous(profile: &Path, active: &str) -> Result<(), String> {
    for entry in fs::read_dir(profile).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name != active
            && valid_directory(&name)
            && entry.path().join(GENERATION_MARKER).is_file()
        {
            remove_private_path(&entry.path())
                .map_err(|e| format!("Cannot remove previous history storage: {e}"))?;
        }
    }
    // The user explicitly chose to discard the previous JSON history. Exports
    // outside these known legacy profile paths are never touched.
    remove_private_path(&profile.join("clipdeck-store.json"))?;
    remove_private_path(&profile.join("images"))?;
    for name in [
        "clipdeck.sqlite3",
        "clipdeck.sqlite3-wal",
        "clipdeck.sqlite3-shm",
    ] {
        remove_private_path(&profile.join(name))?;
    }
    Ok(())
}
fn convert_profile(
    profile: &Path,
    config: &PrivacyConfig,
    enabled: bool,
    key: &EncryptionKey,
) -> Result<PrivacyConfig, String> {
    let source = profile.join(&config.data_dir);
    validate_generation(&source)?;
    let next_name = format!("store-{}", clipdeck_core::store::uuid_v4ish());
    let next = profile.join(&next_name);
    mark_generation(&next)?;
    let desired = PrivacyConfig {
        configured: true,
        encrypted: enabled,
        pending: None,
        data_dir: next_name,
    };
    let result = (|| {
        crypto::export_database(
            &source.join("clipdeck.sqlite3"),
            &next.join("clipdeck.sqlite3"),
            config.encrypted.then_some(key),
            enabled.then_some(key),
        )?;
        for entry in fs::read_dir(source.join("images")).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
                return Err("Image storage contains an unsupported file type".into());
            }
            crypto::copy_image_transform(
                &entry.path(),
                &next.join("images").join(entry.file_name()),
                config.encrypted.then_some(key),
                enabled.then_some(key),
            )
            .map_err(|e| format!("Cannot convert image: {e}"))?;
        }
        File::open(next.join("images"))
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        File::open(&next)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        save_config(profile, &desired)
    })();
    if let Err(error) = result {
        // A directory fsync can fail after the policy rename has committed.
        // Never remove a generation that the active policy already references.
        if load_config(profile).is_ok_and(|current| current.data_dir == desired.data_dir) {
            return Err(error);
        }
        let _ = remove_private_path(&next);
        return Err(error);
    }
    Ok(desired)
}
fn initialize_profile(profile: &Path) -> Result<PrivacyConfig, String> {
    // A missing policy must never make an existing generation disposable.
    for entry in fs::read_dir(profile).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if valid_directory(&entry.file_name().to_string_lossy())
            && entry.path().join(GENERATION_MARKER).exists()
        {
            return Err("Encryption policy is missing for existing history; restore the policy instead of creating an empty store".into());
        }
    }
    let mut config = PrivacyConfig::default();
    let old_db = profile.join("clipdeck.sqlite3");
    if old_db.exists() {
        // Adopt the first SQLite layout without importing or reordering rows.
        config.data_dir = format!("store-{}", clipdeck_core::store::uuid_v4ish());
        let next = profile.join(&config.data_dir);
        mark_generation(&next)?;
        let result = (|| {
            crypto::export_database(&old_db, &next.join("clipdeck.sqlite3"), None, None)?;
            if profile.join("images").exists() {
                real_directory(&profile.join("images"), false)?;
                for entry in fs::read_dir(profile.join("images")).map_err(|e| e.to_string())? {
                    let entry = entry.map_err(|e| e.to_string())?;
                    if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
                        return Err("Cannot adopt an image symlink or directory".into());
                    }
                    crypto::copy_image_transform(
                        &entry.path(),
                        &next.join("images").join(entry.file_name()),
                        None,
                        None,
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
            save_config(profile, &config)
        })();
        if let Err(error) = result {
            if !load_config(profile).is_ok_and(|current| current.data_dir == config.data_dir) {
                let _ = remove_private_path(&next);
            }
            return Err(error);
        }
    } else {
        let directory = profile.join(&config.data_dir);
        if directory.exists()
            && fs::read_dir(&directory)
                .map_err(|e| e.to_string())?
                .next()
                .is_some()
        {
            return Err("Existing storage has no encryption policy".into());
        }
        mark_generation(&directory)?;
        let store = Store::try_open(&directory).map_err(|e| e.to_string())?;
        drop(store);
        save_config(profile, &config)?;
    }
    Ok(config)
}
fn open_with(
    profile: &Path,
    apply_pending: bool,
    provider: &impl Keyring,
) -> Result<(Store, PrivacyConfig), String> {
    fs::create_dir_all(profile).map_err(|e| e.to_string())?;
    let mut config = if profile.join(POLICY_FILE).exists() {
        load_config(profile)?
    } else {
        initialize_profile(profile)?
    };
    validate_generation(&profile.join(&config.data_dir))?;
    if let Some(enabled) = config.pending {
        if !apply_pending {
            return Err("An encryption change is pending; start ClipDeck to finish it".into());
        }
        let key = required_key(provider, profile)?;
        config = convert_profile(profile, &config, enabled, &key)?;
    }
    let key = if config.encrypted {
        Some(required_key(provider, profile)?)
    } else {
        None
    };
    let directory = profile.join(&config.data_dir);
    validate_generation(&directory)?;
    let store = Store::try_open_with_key(&directory, key)
        .map_err(|e| format!("Cannot open history: {e}"))?;
    if apply_pending {
        cleanup_previous(profile, &config.data_dir)?;
    }
    Ok((store, config))
}
fn wait_for_keyring(
    profile: &Path,
    provider: &impl Keyring,
    attempts: usize,
    mut wait: impl FnMut(),
) -> Result<(), String> {
    let mut remaining = attempts.max(1);
    loop {
        match required_key(provider, profile) {
            Ok(_) => return Ok(()),
            Err(error) if remaining == 1 => return Err(format!(
                "{error}. Unlock or restore the desktop keyring, then restart ClipDeck."
            )),
            Err(_) => {
                remaining -= 1;
                wait();
            }
        }
    }
}

pub fn open_store(profile: &Path, apply_pending: bool) -> Result<(Store, PrivacyConfig), String> {
    if apply_pending {
        let config = load_config(profile)?;
        if config.encrypted || config.pending.is_some() {
            // Login can launch the daemon before Secret Service exposes its keys.
            // Keep storage closed and watching stopped until a key is available.
            wait_for_keyring(profile, &DesktopKeyring, KEYRING_STARTUP_ATTEMPTS, || {
                std::thread::sleep(std::time::Duration::from_secs(1));
            })?;
        }
    }
    open_with(profile, apply_pending, &DesktopKeyring)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct StartupKeyring(std::cell::RefCell<std::collections::VecDeque<Result<Option<EncryptionKey>, String>>>);
    impl Keyring for StartupKeyring {
        fn get(&self, _: &Path) -> Result<Option<EncryptionKey>, String> {
            self.0.borrow_mut().pop_front().unwrap_or(Ok(None))
        }
        fn put(&self, _: &Path, _: &EncryptionKey) -> Result<(), String> {
            panic!("startup must never replace an encryption key")
        }
    }

    #[test]
    fn encrypted_startup_waits_for_keyring_readiness() {
        let keys = StartupKeyring(std::cell::RefCell::new(std::collections::VecDeque::from([
            Ok(None), Err("keyring not ready".into()), Ok(Some(EncryptionKey::generate().unwrap())),
        ])));
        let mut waits = 0;
        assert!(wait_for_keyring(Path::new("/synthetic-profile"), &keys, 5, || waits += 1).is_ok());
        assert_eq!(waits, 2);
        assert!(keys.0.borrow().is_empty());
    }

    #[test]
    fn encrypted_startup_retry_is_bounded_and_preserves_missing_key_error() {
        let keys = StartupKeyring(std::cell::RefCell::new(std::collections::VecDeque::new()));
        let mut waits = 0;
        let error = wait_for_keyring(Path::new("/synthetic-profile"), &keys, 3, || waits += 1).unwrap_err();
        assert!(error.contains("desktop-keyring key is missing"));
        assert_eq!(waits, 2);
    }
    struct TestProfile(PathBuf);
    impl TestProfile {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "clipdeck-security-{}",
                clipdeck_core::store::uuid_v4ish()
            ));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for TestProfile {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn first_launch_requires_explicit_choice_and_policy_contains_no_key() {
        let p = TestProfile::new();
        assert!(!load_config(&p.0).unwrap().configured);
        let config = choose_plaintext(&p.0).unwrap();
        assert!(config.configured);
        assert!(!config.encrypted);
        let bytes = fs::read(p.0.join(POLICY_FILE)).unwrap();
        let policy: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(policy.as_object().unwrap().len(), 4);
        assert_eq!(
            fs::metadata(p.0.join(POLICY_FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    #[test]
    fn policy_rejects_escaping_storage_directory() {
        let p = TestProfile::new();
        fs::write(
            p.0.join(POLICY_FILE),
            r#"{"configured":true,"encrypted":false,"pending":null,"data_dir":"../private"}"#,
        )
        .unwrap();
        assert!(load_config(&p.0).is_err());
    }

    struct MemoryKeyring(std::cell::RefCell<Option<EncryptionKey>>);
    impl Keyring for MemoryKeyring {
        fn get(&self, _: &Path) -> Result<Option<EncryptionKey>, String> {
            Ok(self.0.borrow().clone())
        }
        fn put(&self, _: &Path, key: &EncryptionKey) -> Result<(), String> {
            *self.0.borrow_mut() = Some(key.clone());
            Ok(())
        }
    }
    #[test]
    fn generation_conversion_preserves_history_and_external_images_both_directions() {
        let p = TestProfile::new();
        let keys = MemoryKeyring(std::cell::RefCell::new(None));
        let (mut store, _) = open_with(&p.0, false, &keys).unwrap();
        let content = "confidential synthetic history";
        let item=clipdeck_core::ClipItem::from_import(&serde_json::json!({"id":"history","type":"image","content":content,"imageFile":"example.png"})).unwrap();
        let png = include_bytes!("../assets/icon_tray.png").to_vec();
        store.add_item_with_image(item, Some(png.clone()));
        store.save();
        drop(store);
        choose_plaintext(&p.0).unwrap();
        prepare_with(&p.0, true, &keys).unwrap();
        let (store, encrypted) = open_with(&p.0, true, &keys).unwrap();
        assert!(encrypted.encrypted && encrypted.configured && encrypted.pending.is_none());
        assert_eq!(store.find("history").unwrap().content, content);
        assert_eq!(store.image_bytes("example.png").unwrap(), png);
        assert_ne!(fs::read(store.image_dir.join("example.png")).unwrap(), png);
        assert!(!fs::read(store.path())
            .unwrap()
            .starts_with(b"SQLite format 3"));
        assert!(
            !p.0.join("store").exists(),
            "plaintext previous generation must be removed"
        );
        drop(store);
        let key = keys.0.borrow_mut().take().unwrap();
        let before = fs::read(p.0.join(&encrypted.data_dir).join("clipdeck.sqlite3")).unwrap();
        assert!(
            open_with(&p.0, false, &keys).is_err(),
            "missing key must stay locked"
        );
        assert_eq!(
            before,
            fs::read(p.0.join(&encrypted.data_dir).join("clipdeck.sqlite3")).unwrap()
        );
        *keys.0.borrow_mut() = Some(key);
        prepare_with(&p.0, false, &keys).unwrap();
        let (store, plain) = open_with(&p.0, true, &keys).unwrap();
        assert!(!plain.encrypted);
        assert_ne!(plain.data_dir, encrypted.data_dir);
        assert_eq!(store.find("history").unwrap().content, content);
        assert_eq!(fs::read(store.image_dir.join("example.png")).unwrap(), png);
        assert!(fs::read(store.path())
            .unwrap()
            .starts_with(b"SQLite format 3"));
    }
    #[test]
    fn failed_image_conversion_preserves_the_active_generation() {
        let p = TestProfile::new();
        let keys = MemoryKeyring(std::cell::RefCell::new(None));
        let (mut store, _) = open_with(&p.0, false, &keys).unwrap();
        store.add_item(
            clipdeck_core::ClipItem::from_import(
                &serde_json::json!({"id":"kept","type":"text","content":"keep on failure"}),
            )
            .unwrap(),
        );
        std::os::unix::fs::symlink("/does-not-exist", store.image_dir.join("bad.png")).unwrap();
        drop(store);
        choose_plaintext(&p.0).unwrap();
        prepare_with(&p.0, true, &keys).unwrap();
        assert!(open_with(&p.0, true, &keys).is_err());
        let cfg = load_config(&p.0).unwrap();
        assert!(!cfg.encrypted);
        assert_eq!(cfg.data_dir, "store");
        assert_eq!(cfg.pending, Some(true));
        let original = Store::open(&p.0.join("store"));
        assert!(original.contains_id("kept"));
        assert_eq!(
            fs::read_dir(&p.0)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|e| e.file_name().to_string_lossy().starts_with("store-"))
                .count(),
            0
        );
    }
    #[test]
    fn missing_policy_or_database_never_replaces_existing_history() {
        let p = TestProfile::new();
        let keys = MemoryKeyring(std::cell::RefCell::new(None));
        let (store, config) = open_with(&p.0, false, &keys).unwrap();
        drop(store);
        let db = p.0.join(&config.data_dir).join("clipdeck.sqlite3");
        let before = fs::read(&db).unwrap();
        fs::remove_file(p.0.join(POLICY_FILE)).unwrap();
        assert!(open_with(&p.0, true, &keys).is_err());
        assert_eq!(fs::read(&db).unwrap(), before);
        save_config(&p.0, &config).unwrap();
        fs::remove_file(&db).unwrap();
        assert!(open_with(&p.0, true, &keys).is_err());
        assert!(!db.exists());
    }
    #[test]
    fn root_sqlite_history_and_images_are_preserved_when_adopting_generations() {
        let p = TestProfile::new();
        let keys = MemoryKeyring(std::cell::RefCell::new(None));
        let mut store = Store::open(&p.0);
        store.add_item(clipdeck_core::ClipItem::from_import(&serde_json::json!({"id":"preserved","type":"text","content":"existing SQLite history"})).unwrap());
        let image = include_bytes!("../assets/icon_tray.png");
        store.put_image("kept.png", image).unwrap();
        drop(store);
        let (store, config) = open_with(&p.0, true, &keys).unwrap();
        assert!(!config.configured);
        assert!(store.contains_id("preserved"));
        assert_eq!(store.image_bytes("kept.png").unwrap(), image);
        assert!(!p.0.join("clipdeck.sqlite3").exists());
        assert!(!p.0.join("images").exists());
    }
    #[test]
    fn generation_and_image_directory_symlinks_are_rejected() {
        for image_directory in [false, true] {
            let p = TestProfile::new();
            let outside = TestProfile::new();
            let keys = MemoryKeyring(std::cell::RefCell::new(None));
            let (store, config) = open_with(&p.0, false, &keys).unwrap();
            drop(store);
            let storage = p.0.join(&config.data_dir);
            let target = if image_directory {
                storage.join("images")
            } else {
                storage
            };
            fs::remove_dir_all(&target).unwrap();
            std::os::unix::fs::symlink(&outside.0, &target).unwrap();
            assert!(open_with(&p.0, true, &keys).is_err());
            assert_eq!(fs::read_dir(&outside.0).unwrap().count(), 0);
        }
    }
    #[test]
    #[ignore = "requires the user's desktop keyring; run only for explicit local QA"]
    fn desktop_keyring_roundtrip() {
        let p = TestProfile::new();
        let id = account(&p.0).unwrap();
        struct RemoveKey(String);
        impl Drop for RemoveKey {
            fn drop(&mut self) {
                let _ = Command::new("secret-tool")
                    .args(["clear", "application", crate::APP_ID, "profile", &self.0])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
        let _cleanup = RemoveKey(id);
        let key = EncryptionKey::generate().unwrap();
        DesktopKeyring.put(&p.0, &key).unwrap();
        let restored = DesktopKeyring
            .get(&p.0)
            .unwrap()
            .expect("stored key missing");
        assert!(restored.expose_hex().as_bytes() == key.expose_hex().as_bytes());
        let (mut store, _) = open_with(&p.0, false, &DesktopKeyring).unwrap();
        store.settings.watch_enabled = false;
        store.settings.launch_at_login = false;
        store.settings.hooks.clear();
        store.settings.sync_directory.clear();
        store.save();
        drop(store);
        choose_plaintext(&p.0).unwrap();
        prepare_with(&p.0, true, &DesktopKeyring).unwrap();
        let (_store, config) = open_with(&p.0, true, &DesktopKeyring).unwrap();
        assert!(config.encrypted);
    }
}
