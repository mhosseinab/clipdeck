//! Fixture-driven parity tests: every case in spec/fixtures must pass against
//! this crate, exactly as `spec/verify.mjs` validates against the reference.

use clipdeck_core::*;
use serde::Deserialize;
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    // crates/core -> crates -> linux -> apps -> repo root
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("..")
        .join("spec")
        .join("fixtures")
}

fn load<T: for<'de> Deserialize<'de>>(name: &str) -> T {
    let raw = std::fs::read_to_string(fixtures_dir().join(name)).expect(name);
    serde_json::from_str(&raw).expect(name)
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("clipdeck-test-{tag}-{}", store::uuid_v4ish()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/* ---------------- classify-text ---------------- */

#[derive(Deserialize)]
struct ClassifyFx {
    cases: Vec<ClassifyCase>,
}
#[derive(Deserialize)]
struct ClassifyCase {
    id: String,
    raw: String,
    expected: String,
}

#[test]
fn classify_text_fixture() {
    let fx: ClassifyFx = load("classify-text.json");
    for c in fx.cases {
        assert_eq!(classify(&c.raw).as_str(), c.expected, "case {}", c.id);
    }
}

/* ---------------- preview-title ---------------- */

#[derive(Deserialize)]
struct PreviewFx {
    cases: Vec<PreviewCase>,
}
#[derive(Deserialize)]
struct PreviewCase {
    id: String,
    #[serde(rename = "type")]
    item_type: String,
    content: String,
    dims: Option<Dims>,
    expected: ExpectedPair,
}
#[derive(Deserialize)]
struct Dims {
    width: u32,
    height: u32,
}
#[derive(Deserialize)]
struct ExpectedPair {
    title: String,
    preview: String,
}

fn item_type_of(s: &str) -> ItemType {
    match s {
        "text" => ItemType::Text,
        "link" => ItemType::Link,
        "code" => ItemType::Code,
        "color" => ItemType::Color,
        "image" => ItemType::Image,
        "file" => ItemType::File,
        "html" => ItemType::Html,
        other => panic!("unknown type {other}"),
    }
}

#[test]
fn preview_title_fixture() {
    let fx: PreviewFx = load("preview-title.json");
    for c in fx.cases {
        let dims = c.dims.map(|d| (d.width, d.height));
        let (title, preview) = derive_pair(item_type_of(&c.item_type), &c.content, dims);
        assert_eq!(title, c.expected.title, "case {} title", c.id);
        assert_eq!(preview, c.expected.preview, "case {} preview", c.id);
    }
}

/* ---------------- store-rules ---------------- */

#[derive(Deserialize)]
struct StoreFx {
    #[serde(rename = "maxItems")]
    max_items_field_marker: Option<u32>,
    cases: Vec<StoreCase>,
    #[serde(rename = "importCases")]
    import_cases: Vec<ImportCase>,
}
#[derive(Deserialize)]
struct StoreCase {
    id: String,
    #[serde(rename = "maxItems")]
    max_items: u32,
    adds: Vec<Add>,
    reorder: Option<Vec<String>>,
    expected: StoreExpected,
}
#[derive(Deserialize)]
struct Add {
    content: String,
    #[serde(rename = "type")]
    item_type: Option<String>,
    pinned: Option<bool>,
    #[serde(rename = "pinboardId")]
    pinboard_id: Option<String>,
    #[serde(rename = "stackOrder")]
    stack_order: Option<i64>,
}
#[derive(Deserialize)]
struct StoreExpected {
    contents: Option<Vec<String>>,
    count: Option<usize>,
    stacked_count: Option<usize>,
    imported: Option<usize>,
    skipped: Option<usize>,
}
#[derive(Deserialize)]
struct ImportCase {
    id: String,
    local: Vec<serde_json::Value>,
    #[serde(rename = "import")]
    import_items: Vec<serde_json::Value>,
    expected: StoreExpected,
}

fn make_item(n: usize, a: &Add) -> ClipItem {
    ClipItem {
        id: format!("gen-{n}"),
        item_type: item_type_of(a.item_type.as_deref().unwrap_or("text")),
        content: a.content.clone(),
        preview: a.content.clone(),
        title: a.content.clone(),
        source_app: None,
        created_at: n as i64,
        size_bytes: a.content.len() as u64,
        pinned: a.pinned.unwrap_or(false),
        pinboard_id: a.pinboard_id.clone(),
        stack_order: a.stack_order,
        image_file: None,
    }
}

#[test]
fn store_rules_fixture() {
    let fx: StoreFx = load("store-rules.json");
    let _ = fx.max_items_field_marker;
    for c in fx.cases {
        let dir = tmpdir("store");
        let mut store = Store::open(&dir);
        store.settings.max_items = c.max_items;
        for (n, a) in c.adds.iter().enumerate() {
            store.add_item(make_item(n, a));
        }
        if let Some(order) = &c.reorder {
            // map content -> id of the item currently holding it (fixture has unique contents)
            let ids: Vec<String> = order
                .iter()
                .filter_map(|content| {
                    store.items.iter().find(|i| &i.content == content).map(|i| i.id.clone())
                })
                .collect();
            store.reorder(&ids);
        }
        if let Some(expected) = c.expected.contents.as_ref() {
            let got: Vec<String> = store.items.iter().map(|i| i.content.clone()).collect();
            assert_eq!(&got, expected, "case {} contents", c.id);
        }
        if let Some(count) = c.expected.count {
            assert_eq!(store.items.len(), count, "case {} count", c.id);
        }
        if let Some(sc) = c.expected.stacked_count {
            let got = store.items.iter().filter(|i| i.stack_order.is_some()).count();
            assert_eq!(got, sc, "case {} stacked", c.id);
        }
    }
}

#[test]
fn store_import_fixture() {
    let fx: StoreFx = load("store-rules.json");
    for c in fx.import_cases {
        let dir = tmpdir("import");
        let mut store = Store::open(&dir);
        for (n, local) in c.local.iter().enumerate() {
            let mut item = make_item(n, &Add {
                content: local["content"].as_str().unwrap().into(),
                item_type: local["type"].as_str().map(String::from),
                pinned: None,
                pinboard_id: None,
                stack_order: None,
            });
            if let Some(id) = local.get("id").and_then(|v| v.as_str()) {
                item.id = id.into();
            }
            store.add_item(item);
        }
        let counts = store.import_json(&serde_json::json!({"items": c.import_items}).to_string());
        assert_eq!(counts.imported, c.expected.imported.unwrap(), "case {} imported", c.id);
        assert_eq!(counts.skipped, c.expected.skipped.unwrap(), "case {} skipped", c.id);
        assert_eq!(store.items.len(), c.expected.count.unwrap(), "case {} count", c.id);
        if let Some(sc) = c.expected.stacked_count {
            let got = store.items.iter().filter(|i| i.stack_order.is_some()).count();
            assert_eq!(got, sc, "case {} stacked", c.id);
        }
    }
}

/* ---------------- sync-merge ---------------- */

#[derive(Deserialize)]
struct SyncFx {
    cases: Vec<SyncCase>,
}
#[derive(Deserialize)]
struct SyncCase {
    id: String,
    local: Vec<serde_json::Value>,
    remote: Vec<serde_json::Value>,
    #[serde(rename = "mergeTwice")]
    merge_twice: Option<bool>,
    expected: SyncExpected,
}
#[derive(Deserialize)]
struct SyncExpected {
    imported: Option<usize>,
    skipped: Option<usize>,
    #[serde(rename = "firstImported")]
    first_imported: Option<usize>,
    #[serde(rename = "secondImported")]
    second_imported: Option<usize>,
    contents: Option<Vec<String>>,
    stacked_count: Option<usize>,
}

#[test]
fn sync_merge_fixture() {
    let fx: SyncFx = load("sync-merge.json");
    for c in fx.cases {
        let dir = tmpdir("sync");
        let shared = dir.join("shared");
        let mut store = Store::open(&dir.join("a"));
        for (n, local) in c.local.iter().enumerate() {
            let mut item = make_item(n, &Add {
                content: local["content"].as_str().unwrap().into(),
                item_type: local["type"].as_str().map(String::from),
                pinned: None,
                pinboard_id: None,
                stack_order: None,
            });
            item.id = local["id"].as_str().unwrap().into();
            item.created_at = local["createdAt"].as_i64().unwrap_or(n as i64);
            store.add_item(item);
        }
        let mut sync = SyncStore::new(shared.to_str().unwrap());
        // remote rows written as JSONL {"v":1,"item":...}
        let file = shared.join("clipdeck-sync.jsonl");
        let mut lines = String::new();
        for row in &c.remote {
            lines.push_str(&serde_json::json!({"v": 1, "item": row}).to_string());
            lines.push('\n');
        }
        std::fs::write(&file, lines).unwrap();
        let counts = sync.merge(&mut store);
        if let Some(true) = c.merge_twice {
            assert_eq!(counts.imported, c.expected.first_imported.unwrap(), "{} first", c.id);
            let second = sync.merge(&mut store);
            assert_eq!(second.imported, c.expected.second_imported.unwrap(), "{} second", c.id);
        } else {
            assert_eq!(counts.imported, c.expected.imported.unwrap(), "{} imported", c.id);
        }
        if let Some(sk) = c.expected.skipped {
            assert_eq!(counts.skipped, sk, "{} skipped", c.id);
        }
        if let Some(expected) = c.expected.contents.as_ref() {
            let got: Vec<String> = store.items.iter().map(|i| i.content.clone()).collect();
            assert_eq!(&got, expected, "{} contents", c.id);
        }
        if let Some(sc) = c.expected.stacked_count {
            let got = store.items.iter().filter(|i| i.stack_order.is_some()).count();
            assert_eq!(got, sc, "{} stacked", c.id);
        }
    }
}

/* ---------------- hooks ---------------- */

#[derive(Deserialize)]
struct HooksFx {
    parsing: Vec<HookCase>,
}
#[derive(Deserialize)]
struct HookCase {
    id: String,
    line: String,
    expected: Option<HookExpected>,
}
#[derive(Deserialize)]
struct HookExpected {
    event: String,
    command: String,
}

#[test]
fn hooks_fixture() {
    let fx: HooksFx = load("hooks.json");
    for c in fx.parsing {
        let got = parse_hook_line(&c.line);
        match c.expected {
            None => assert!(got.is_none(), "case {}", c.id),
            Some(e) => {
                let got = got.expect(&c.id);
                assert_eq!(got.event, e.event, "case {}", c.id);
                assert_eq!(got.command, e.command, "case {}", c.id);
            }
        }
    }
}

/* ---------------- accelerators ---------------- */

#[derive(Deserialize)]
struct AccelFx {
    recording: Vec<RecCase>,
    display: Vec<DispCase>,
}
#[derive(Deserialize)]
struct RecCase {
    id: String,
    event: RecEvent,
    expected: Option<String>,
}
#[derive(Deserialize)]
struct RecEvent {
    ctrl: bool,
    meta: bool,
    alt: bool,
    shift: bool,
    code: String,
}
#[derive(Deserialize)]
struct DispCase {
    accel: String,
    pretty: String,
}

#[test]
fn accelerators_fixture() {
    let fx: AccelFx = load("accelerators.json");
    for c in fx.recording {
        let ev = KeyPress {
            ctrl: c.event.ctrl,
            meta: c.event.meta,
            alt: c.event.alt,
            shift: c.event.shift,
            code: c.event.code.clone(),
        };
        assert_eq!(record_accel(&ev), c.expected, "case {}", c.id);
    }
    for c in fx.display {
        assert_eq!(pretty_accel(&c.accel), c.pretty, "display {}", c.accel);
    }
}

/* ---------------- autostart ---------------- */

#[derive(Deserialize)]
struct AutostartFx {
    cases: Vec<AsCase>,
}
#[derive(Deserialize)]
struct AsCase {
    id: String,
    platform: String,
    enable: bool,
    #[serde(rename = "execPath")]
    exec_path: String,
    #[serde(rename = "appImage")]
    app_image: Option<String>,
    #[serde(rename = "expectedFile")]
    expected_file: Option<AsExpectedFile>,
    #[serde(rename = "expectedLoginItemSettings")]
    expected_login: Option<bool>,
}
#[derive(Deserialize)]
struct AsExpectedFile {
    content: String,
}

#[test]
fn autostart_fixture() {
    let fx: AutostartFx = load("autostart.json");
    for c in fx.cases {
        if c.platform == "linux" {
            let home = tmpdir("autostart");
            let exe = c.app_image.as_deref().unwrap_or(&c.exec_path);
            apply_login_item_linux(c.enable, exe, &home).unwrap();
            let file = autostart_file(&home);
            match &c.expected_file {
                Some(exp) => {
                    assert_eq!(std::fs::read_to_string(&file).unwrap(), exp.content, "case {}", c.id);
                }
                None => assert!(!file.exists(), "case {} should remove entry", c.id),
            }
        } else {
            // non-Linux: fixture records the boolean the OS API receives
            assert!(c.expected_login.is_some(), "case {}", c.id);
        }
    }
}
