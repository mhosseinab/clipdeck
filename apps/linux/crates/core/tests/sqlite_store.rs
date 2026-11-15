use clipdeck_core::{ClipItem, ItemType, Store};
use serde_json::json;
use std::path::PathBuf;

struct Profile(PathBuf);
impl Profile {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "clipdeck-sqlite-{}",
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
fn item(id: usize, content: &str) -> ClipItem {
    ClipItem {
        id: format!("clip-{id}"),
        item_type: ItemType::Text,
        content: content.into(),
        title: content.into(),
        preview: content.into(),
        source_app: None,
        created_at: id as i64,
        size_bytes: content.len() as u64,
        pinned: false,
        pinboard_id: None,
        stack_order: None,
        image_file: None,
    }
}
#[test]
fn legacy_json_is_not_loaded_into_new_native_store() {
    let profile = Profile::new();
    let old = json!({"items": [item(1,"old content")], "pinboards": [], "settings": {"watchEnabled":false}});
    std::fs::write(profile.0.join("clipdeck-store.json"), old.to_string()).unwrap();
    let store = Store::open(&profile.0);
    assert_eq!(
        store.len(),
        0,
        "legacy history must not be migrated implicitly"
    );
    assert!(profile.0.join("clipdeck.sqlite3").exists());
}
#[test]
fn sqlite_roundtrip_and_reader_return_bounded_pages_without_truncating_copy_content() {
    use clipdeck_core::{ClipFilter, StoreReader};
    let profile = Profile::new();
    let mut store = Store::open(&profile.0);
    store.settings.max_items = 1000;
    let body = "🦀".repeat(5000);
    for n in 0..350 {
        store.add_item(item(n, &format!("{n} {body}")));
    }
    let reader = StoreReader::open(store.path()).unwrap();
    let page = reader.page("", &ClipFilter::All, 300, 10000).unwrap();
    assert_eq!(page.total, 350);
    assert_eq!(page.items.len(), 50);
    assert_eq!(page.items[0].id, "clip-49");
    assert_eq!(page.items.last().unwrap().id, "clip-0");
    assert!(page.items.iter().all(|i| i.content.chars().count() <= 4096
        && i.title.chars().count() <= 256
        && i.preview.chars().count() <= 512));
    assert_eq!(store.find("clip-0").unwrap().content, format!("0 {body}"));
    assert_eq!(
        store.page("", &ClipFilter::All, 0, usize::MAX).items.len(),
        64
    );
    store.save();
    drop(store);
    assert_eq!(Store::open(&profile.0).len(), 350);
}
#[test]
fn unicode_substrings_short_queries_and_literal_wildcards_keep_rust_semantics() {
    use clipdeck_core::ClipFilter;
    let p = Profile::new();
    let mut store = Store::open(&p.0);
    for (n, text) in ["İstanbul", "ΟΣ", "a%b_c", "🦀 Rust"].iter().enumerate() {
        store.add_item(item(n, text));
    }
    for query in ["i\u{307}", "ος", "%", "_", "🦀", "ru", "x"] {
        let expected = store
            .all_items()
            .iter()
            .filter(|i| i.content.to_lowercase().contains(&query.to_lowercase()))
            .count();
        assert_eq!(
            store.page(query, &ClipFilter::All, 0, 64).total,
            expected,
            "{query:?}"
        );
    }
}
#[test]
fn malformed_stream_import_is_atomic_and_roundtrip_export_streams_all_rows() {
    let p = Profile::new();
    let mut store = Store::open(&p.0);
    store.add_item(item(1, "existing"));
    assert!(store
        .import_from_reader(br#"{"items":[{"id":"new","type":"text","content":"new"},"#.as_slice())
        .is_err());
    assert_eq!(store.len(), 1);
    assert!(!store.contains_id("new"));
    let mut out = Vec::new();
    store.export_to_writer(&mut out).unwrap();
    let other = Profile::new();
    let mut dest = Store::open(&other.0);
    let counts = dest.import_from_reader(out.as_slice()).unwrap();
    assert_eq!(counts.imported, 1);
    assert_eq!(dest.find("clip-1").unwrap().content, "existing");
}
#[test]
fn image_cleanup_respects_shared_references_and_never_follows_unsafe_names() {
    let p = Profile::new();
    let mut store = Store::open(&p.0);
    store.settings.max_items = 1;
    let image = store.image_dir.join("shared.png");
    std::fs::write(&image, b"image test data").unwrap();
    let mut a = item(1, "a");
    a.image_file = Some("shared.png".into());
    a.pinned = true;
    let mut b = item(2, "b");
    b.image_file = a.image_file.clone();
    b.pinned = true;
    store.add_item(a);
    store.add_item(b);
    store.remove("clip-1");
    assert!(image.exists());
    store.remove("clip-2");
    assert!(!image.exists());
    let outside = p.0.join("must-stay");
    std::fs::write(&outside, b"outside").unwrap();
    let mut unsafe_item = item(3, "unsafe");
    unsafe_item.image_file = Some("../must-stay".into());
    store.add_item(unsafe_item);
    store.remove("clip-3");
    assert!(outside.exists());
}
#[test]
fn corrupt_sqlite_is_reported_instead_of_resetting_history() {
    let p = Profile::new();
    std::fs::write(p.0.join("clipdeck.sqlite3"), b"not a database").unwrap();
    assert!(Store::try_open(&p.0).is_err());
    assert_eq!(
        std::fs::read(p.0.join("clipdeck.sqlite3")).unwrap(),
        b"not a database"
    );
}
#[test]
fn native_profile_and_database_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let p = Profile::new();
    let store = Store::open(&p.0);
    assert_eq!(
        std::fs::metadata(&p.0).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(store.path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}
#[test]
fn storage_policy_fixture_matches_native_pages_and_unicode_summaries() {
    use clipdeck_core::{ClipFilter, DISPLAY_CONTENT_CHARS, PAGE_SIZE, SQLITE_CACHE_KIB};
    let fixture: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../../spec/fixtures/storage-policy.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(PAGE_SIZE, fixture["pageSize"].as_u64().unwrap() as usize);
    assert_eq!(
        DISPLAY_CONTENT_CHARS,
        fixture["contentSummaryChars"].as_u64().unwrap() as usize
    );
    assert_eq!(
        SQLITE_CACHE_KIB,
        fixture["sqliteCacheKiB"].as_u64().unwrap() as usize
    );
    for case in fixture["cases"].as_array().unwrap() {
        let p = Profile::new();
        let mut store = Store::open(&p.0);
        assert_eq!(
            store.path().file_name().unwrap().to_str().unwrap(),
            fixture["nativeFile"].as_str().unwrap()
        );
        let count = case["itemCount"].as_u64().unwrap() as usize;
        for n in (0..count).rev() {
            store.add_item(item(n, &format!("entry-{n}")));
        }
        let page = store.page(
            "",
            &ClipFilter::All,
            case["offset"].as_u64().unwrap() as usize,
            case["requestedLimit"].as_u64().unwrap() as usize,
        );
        assert_eq!(page.total, count);
        assert_eq!(
            page.items.len(),
            case["expected"]["count"].as_u64().unwrap() as usize
        );
        for (got, key) in [
            (page.items.first(), "firstIndex"),
            (page.items.last(), "lastIndex"),
        ] {
            let expected = case["expected"][key].as_u64().map(|n| format!("clip-{n}"));
            assert_eq!(got.map(|i| i.id.clone()), expected, "{} {key}", case["id"]);
        }
    }
    for case in fixture["summaryCases"].as_array().unwrap() {
        let p = Profile::new();
        let mut store = Store::open(&p.0);
        let text = case["text"]
            .as_str()
            .unwrap()
            .repeat(case["repeat"].as_u64().unwrap() as usize);
        store.add_item(item(1, &text));
        let shown = store.recent(1).pop().unwrap();
        for (value, key) in [
            (&shown.content, "contentChars"),
            (&shown.title, "titleChars"),
            (&shown.preview, "previewChars"),
        ] {
            assert_eq!(
                value.chars().count(),
                case["expected"][key].as_u64().unwrap() as usize
            );
        }
        assert_eq!(store.find("clip-1").unwrap().content, text);
    }
}

#[test]
fn stack_operations_return_bounded_summaries_but_peek_loads_full_payload() {
    use clipdeck_core::StackDir;
    let profile = Profile::new();
    let mut store = Store::open(&profile.0);
    let content = "large clipboard body".repeat(1000);
    store.add_item(item(1, &content));
    let stacked = store.push_stack("clip-1").unwrap();
    assert!(
        stacked.content.chars().count() <= 4096,
        "stack metadata loaded full payload"
    );
    assert_eq!(store.peek_stack(StackDir::Fifo).unwrap().content, content);
    assert!(store.unstack("clip-1").unwrap().content.chars().count() <= 4096);
}

#[test]
fn bad_utf8_sync_record_does_not_block_later_complete_records() {
    let p = Profile::new();
    let shared = p.0.join("shared");
    let mut sync = clipdeck_core::SyncStore::new(shared.to_str().unwrap());
    let mut bytes = vec![0xff, b'\n'];
    bytes.extend_from_slice(
        br#"{"v":1,"item":{"id":"after-invalid","type":"text","content":"good"}}
"#,
    );
    std::fs::write(shared.join("clipdeck-sync.jsonl"), bytes).unwrap();
    let mut store = Store::open(&p.0.join("profile"));
    let counts = sync.merge(&mut store);
    assert_eq!(counts.imported, 1);
    assert_eq!(counts.skipped, 1);
    assert_eq!(sync.merge(&mut store).imported, 0);
}

#[test]
fn read_page_snapshot_remains_coherent_during_capture_and_deletion() {
    use clipdeck_core::{ClipFilter, StoreReader};
    let profile = Profile::new();
    let mut store = Store::open(&profile.0);
    store.settings.max_items = 1000;
    for n in 0..120 {
        store.add_item(item(n, &format!("content-{n}")));
    }
    let reader = StoreReader::open(store.path()).unwrap();
    let writer = std::thread::spawn(move || {
        for n in 0..50 {
            store.remove(&format!("clip-{n}"));
            store.add_item(item(n + 120, &format!("content-{}", n + 120)));
        }
    });
    for _ in 0..50 {
        let page = reader.page("", &ClipFilter::All, 112, 64).unwrap();
        assert_eq!(page.items.len(), page.total.saturating_sub(112).min(64));
    }
    writer.join().unwrap();
}

#[test]
fn encrypted_store_and_external_images_require_the_matching_key() {
    use clipdeck_core::{ClipFilter, EncryptionKey, StoreReader};
    let profile = Profile::new();
    let key = EncryptionKey::generate().unwrap();
    let mut store = Store::try_open_with_key(&profile.0, Some(key.clone())).unwrap();
    let png = include_bytes!("../../app/assets/icon_tray.png");
    let mut picture = item(1, "encrypted-image.png");
    picture.image_file = Some("encrypted-image.png".into());
    store.add_item_with_image(picture, Some(png.to_vec()));
    store.add_item(item(2, "private clipboard marker"));
    let raw = std::fs::read(store.image_dir.join("encrypted-image.png")).unwrap();
    assert_ne!(raw.as_slice(), png);
    assert!(!raw.starts_with(b"\x89PNG"));
    assert_eq!(store.image_bytes("encrypted-image.png").unwrap(), png);
    let reader = StoreReader::open_with_key(store.path(), Some(&key)).unwrap();
    assert_eq!(
        reader
            .page("marker", &ClipFilter::All, 0, 64)
            .unwrap()
            .total,
        1
    );
    assert!(StoreReader::open(store.path()).is_err());
    drop(reader);
    drop(store);
    assert!(!std::fs::read(profile.0.join("clipdeck.sqlite3"))
        .unwrap()
        .starts_with(b"SQLite format 3"));
    assert!(Store::try_open(&profile.0).is_err());
    assert!(
        Store::try_open_with_key(&profile.0, Some(EncryptionKey::generate().unwrap())).is_err()
    );
    assert_eq!(
        Store::try_open_with_key(&profile.0, Some(key))
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn image_api_uses_file_revisions_and_rejects_escaping_paths() {
    let profile = Profile::new();
    let mut store = Store::open(&profile.0);
    store.put_image("sample.png", b"first").unwrap();
    let before = store.image_info("sample.png").unwrap();
    store.put_image("sample.png", b"other").unwrap();
    let after = store.image_info("sample.png").unwrap();
    assert_eq!(before.0, after.0);
    assert_ne!(before.1, after.1);
    assert_eq!(store.image_bytes("sample.png").unwrap(), b"other");
    assert!(store.put_image("../outside", b"no").is_err());
    assert!(store.image_bytes("../outside").is_none());
    assert!(store.image_info("../outside").is_none());
}
