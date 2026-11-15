//! Native-format checks. Legacy profiles are intentionally not migrated.
use clipdeck_core::{ClipItem, Settings, Store};
use serde_json::json;
use std::path::PathBuf;
struct Profile(PathBuf);
impl Profile {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "clipdeck-upgrade-{}",
            clipdeck_core::store::uuid_v4ish()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn legacy_json_is_left_untouched_and_not_migrated() {
    let p = Profile::new();
    let raw=json!({"items":[{"id":"legacy","type":"text","content":"old"}],"pinboards":[],"settings":{"watchEnabled":false}}).to_string();
    std::fs::write(p.0.join("clipdeck-store.json"), &raw).unwrap();
    let store = Store::open(&p.0);
    assert!(store.is_empty());
    assert_eq!(store.settings, Settings::default());
    assert_eq!(
        std::fs::read_to_string(p.0.join("clipdeck-store.json")).unwrap(),
        raw
    );
}
#[test]
fn sqlite_profile_roundtrip_preserves_settings_items_order_and_images() {
    let p = Profile::new();
    let mut store = Store::open(&p.0);
    store.settings.watch_enabled = false;
    store.settings.launch_at_login = false;
    store
        .settings
        .extra
        .insert("futureSetting".into(), json!({"nested":[true,"retained"]}));
    let board = store.add_pinboard("Notes");
    let mut item = ClipItem::from_import(
        &json!({"id":"image","type":"image","content":"sample.png","imageFile":"sample.png"}),
    )
    .unwrap();
    item.pinboard_id = Some(board.id);
    item.stack_order = Some(2);
    item.pinned = true;
    let png = include_bytes!("../../app/assets/icon_tray.png");
    std::fs::write(store.image_dir.join("sample.png"), png).unwrap();
    store.add_item(item.clone());
    store.save();
    let settings = store.settings.clone();
    drop(store);
    let reopened = Store::open(&p.0);
    assert_eq!(reopened.find("image"), Some(item));
    assert_eq!(reopened.settings, settings);
    assert_eq!(
        std::fs::read(reopened.image_dir.join("sample.png")).unwrap(),
        png
    );
}
