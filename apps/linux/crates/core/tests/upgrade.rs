//! Existing-profile upgrade checks; never touch the user's clipboard or profile.

use clipdeck_core::{types::StoreFile, Settings, Store};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

struct ProfileTest(PathBuf);

impl ProfileTest {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("clipdeck-upgrade-{}", clipdeck_core::store::uuid_v4ish()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn profile(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::create_dir_all(path.join("images")).unwrap();
        path
    }
}

impl Drop for ProfileTest {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sample() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../spec/examples/sample-store.json");
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn missing_legacy_settings_keys_keep_history_and_use_defaults() {
    let test = ProfileTest::new();
    let profile = test.profile("legacy");
    let mut original = sample();
    original["settings"] = json!({"watchEnabled": false, "futureSetting": {"value": 7}});
    std::fs::write(profile.join("clipdeck-store.json"), original.to_string()).unwrap();

    let store = Store::open(&profile);
    assert_eq!(store.items.len(), original["items"].as_array().unwrap().len(), "missing settings discarded history");
    assert_eq!(store.pinboards.len(), original["pinboards"].as_array().unwrap().len());
    let mut expected = Settings::default();
    expected.watch_enabled = false;
    expected.extra.insert("futureSetting".into(), json!({"value": 7}));
    assert_eq!(store.settings, expected);
    store.save();
    let reopened = Store::open(&profile);
    assert_eq!(reopened.items, store.items);
    assert_eq!(reopened.pinboards, store.pinboards);
    assert_eq!(reopened.settings, expected);
}

#[test]
fn full_profile_backup_upgrade_and_restore_preserve_data_and_images() {
    let test = ProfileTest::new();
    let original_profile = test.profile("original");
    let backup = test.profile("backup");
    let upgraded = test.profile("upgraded");
    let restored = test.profile("restored");
    let mut original = sample();
    original["settings"]["watchEnabled"] = json!(false);
    original["settings"]["launchAtLogin"] = json!(false);
    original["settings"]["futureSetting"] = json!({"nested": [true, "retained"]});
    original["items"].as_array_mut().unwrap().push(json!({
        "id": "legacy-image", "type": "image", "content": "existing.png",
        "title": "Existing image", "preview": "Existing image", "imageFile": "existing.png",
        "sourceApp": "Synthetic source", "createdAt": 1770000000000_i64,
        "sizeBytes": 0, "pinned": true, "pinboardId": "pb-sample-1", "stackOrder": 1
    }));
    let original_bytes = serde_json::to_vec(&original).unwrap();
    let image_bytes = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../app/assets/icon_tray.png")).unwrap();
    std::fs::write(original_profile.join("clipdeck-store.json"), &original_bytes).unwrap();
    std::fs::write(original_profile.join("images/existing.png"), &image_bytes).unwrap();
    for (source, destination) in [(&original_profile, &backup), (&backup, &upgraded)] {
        std::fs::copy(source.join("clipdeck-store.json"), destination.join("clipdeck-store.json")).unwrap();
        std::fs::copy(source.join("images/existing.png"), destination.join("images/existing.png")).unwrap();
    }

    let expected: StoreFile = serde_json::from_value(original).unwrap();
    let store = Store::open(&upgraded);
    assert_eq!(store.items, expected.items);
    assert_eq!(store.pinboards, expected.pinboards);
    assert_eq!(store.settings, expected.settings);
    store.save();
    let reopened = Store::open(&upgraded);
    assert_eq!(reopened.items, expected.items);
    assert_eq!(reopened.pinboards, expected.pinboards);
    assert_eq!(reopened.settings, expected.settings);
    assert_eq!(std::fs::read(upgraded.join("images/existing.png")).unwrap(), image_bytes);
    assert_eq!(std::fs::read(backup.join("clipdeck-store.json")).unwrap(), original_bytes);

    std::fs::copy(backup.join("clipdeck-store.json"), restored.join("clipdeck-store.json")).unwrap();
    std::fs::copy(backup.join("images/existing.png"), restored.join("images/existing.png")).unwrap();
    let restored_store = Store::open(&restored);
    assert_eq!(restored_store.items, expected.items);
    assert_eq!(restored_store.pinboards, expected.pinboards);
    assert_eq!(restored_store.settings, expected.settings);
    assert_eq!(std::fs::read(restored.join("images/existing.png")).unwrap(), image_bytes);
}
