//! Independent fixture oracle. This binary deliberately does not depend on
//! clipdeck-core: fixture consistency and runtime parity are separate checks.

use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

type Result<T> = std::result::Result<T, String>;

#[derive(Default, Debug)]
struct Checks {
    count: usize,
    failures: usize,
}

impl Checks {
    fn check(&mut self, condition: bool, label: impl AsRef<str>) {
        self.count += 1;
        if !condition {
            self.failures += 1;
            eprintln!("FAIL: {}", label.as_ref());
        }
    }

    fn equal(&mut self, actual: Value, expected: &Value, label: impl AsRef<str>) {
        self.check(
            actual == *expected,
            format!("{}: got {actual}, want {expected}", label.as_ref()),
        );
    }
}

fn load_json(root: &Path, relative: &str) -> Result<Value> {
    let path = root.join(relative);
    let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))
}

fn array<'a>(value: &'a Value, key: &str) -> Result<&'a [Value]> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| format!("expected array field {key}"))
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("expected string field {key}"))
}

fn ecma_whitespace(ch: char) -> bool {
    matches!(ch, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}'
        | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}'
        | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

fn trim(text: &str) -> &str {
    text.trim_matches(ecma_whitespace)
}

fn classify(raw: &str) -> &'static str {
    let text = trim(raw);
    let hex = text.strip_prefix('#').is_some_and(|digits| {
        (3..=8).contains(&digits.len()) && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    });
    if hex || text.starts_with("rgb(") || text.starts_with("rgba(") {
        return "color";
    }
    let link = ["http://", "https://", "www."].iter().any(|prefix| {
        text.strip_prefix(prefix)
            .is_some_and(|rest| !rest.is_empty() && !rest.chars().any(ecma_whitespace))
    });
    if link {
        return "link";
    }
    let punctuation = text.chars().any(|ch| ";{}()[]".contains(ch));
    let keyword = [
        "const ",
        "let ",
        "var ",
        "function ",
        "def ",
        "class ",
        "import ",
        "#include",
        "</",
        "/>",
    ]
    .iter()
    .any(|word| text.contains(word));
    let indentation = text.contains('\n') && (raw.contains("  ") || raw.contains('\t'));
    if u8::from(punctuation) + u8::from(keyword) + u8::from(indentation) >= 2 {
        "code"
    } else {
        "text"
    }
}

fn truncate(text: &str, cap: usize) -> String {
    if text.chars().count() <= cap {
        text.to_owned()
    } else {
        text.chars().take(cap).collect::<String>() + "…"
    }
}

fn derive(kind: &str, content: &str, dimensions: &Value) -> (String, String) {
    let shared = match kind {
        "link" => Some(match url::Url::parse(content) {
            Ok(url) => url
                .host_str()
                .unwrap_or("")
                .strip_prefix("www.")
                .unwrap_or(url.host_str().unwrap_or(""))
                .to_owned(),
            Err(_) => truncate(content, 80),
        }),
        "color" => Some(trim(content).to_owned()),
        "file" => {
            let paths: Vec<_> = content.split('\n').collect();
            Some(if paths.len() == 1 {
                paths[0].rsplit('/').next().unwrap_or("").to_owned()
            } else {
                format!("{} files", paths.len())
            })
        }
        "image" => {
            return (
                "Image".into(),
                format!(
                    "{}×{} image",
                    display_field(dimensions, "width"),
                    display_field(dimensions, "height")
                ),
            )
        }
        _ => None,
    };
    if let Some(label) = shared {
        return (label.clone(), label);
    }
    let first = trim(content.split('\n').next().unwrap_or(""));
    let title = truncate(first, 60);
    (
        if title.is_empty() {
            kind.to_owned()
        } else {
            title
        },
        truncate(first, 120),
    )
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(on) => *on,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn display_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .map(|item| {
                if item.is_null() {
                    String::new()
                } else {
                    display_value(item)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
        _ => value.to_string(),
    }
}

fn display_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .map(display_value)
        .unwrap_or_else(|| "undefined".into())
}

fn item_key(item: &Value) -> String {
    format!(
        "{}|{}",
        display_field(item, "type"),
        display_field(item, "content")
    )
}

fn protected(item: &Value) -> bool {
    truthy(&item["pinned"]) || !item["pinboardId"].is_null() || !item["stackOrder"].is_null()
}

fn simulate_store(case: &Value) -> Result<Vec<Value>> {
    let max = case
        .get("maxItems")
        .and_then(Value::as_u64)
        .ok_or("expected nonnegative maxItems")? as usize;
    let mut items: Vec<Value> = Vec::new();
    let mut next_id = 0;
    for add in array(case, "adds")? {
        let mut item = Map::new();
        let id = match add.get("id").filter(|id| !id.is_null()) {
            Some(id) => id.clone(),
            None => {
                let id = json!(format!("gen-{next_id}"));
                next_id += 1;
                id
            }
        };
        item.insert("id".into(), id);
        for key in ["type", "content"] {
            if let Some(value) = add.get(key) {
                item.insert(key.into(), value.clone());
            }
        }
        item.insert("createdAt".into(), json!(next_id));
        item.insert("pinned".into(), json!(truthy(&add["pinned"])));
        for key in ["pinboardId", "stackOrder"] {
            item.insert(key.into(), add[key].clone());
        }
        let item = Value::Object(item);
        if let Some(index) = items.iter().position(|existing| {
            existing.get("type") == item.get("type")
                && existing.get("content") == item.get("content")
                && existing["stackOrder"].is_null()
        }) {
            let existing = items.remove(index);
            items.insert(0, existing);
        } else {
            items.insert(0, item);
        }
        let mut budget = max.saturating_sub(items.iter().filter(|item| protected(item)).count());
        items.retain(|item| {
            if protected(item) {
                true
            } else if budget > 0 {
                budget -= 1;
                true
            } else {
                false
            }
        });
    }
    if let Some(order) = case.get("reorder").filter(|value| truthy(value)) {
        let order = order.as_array().ok_or("expected reorder array")?;
        // Match insertion-ordered Map semantics, including duplicate content keys.
        let mut remaining: Vec<(Option<Value>, Value)> = Vec::new();
        for item in items {
            let content = item.get("content").cloned();
            if let Some((_, value)) = remaining.iter_mut().find(|(key, _)| *key == content) {
                *value = item;
            } else {
                remaining.push((content, item));
            }
        }
        items = Vec::new();
        for content in order {
            if let Some(index) = remaining
                .iter()
                .position(|(key, _)| key.as_ref() == Some(content))
            {
                items.push(remaining.remove(index).1);
            }
        }
        items.extend(remaining.into_iter().map(|(_, item)| item));
    }
    Ok(items)
}

struct Merge {
    items: Vec<Value>,
    imported: usize,
    skipped: usize,
}

fn reset_stack(raw: &Value) -> Value {
    let mut item = raw.as_object().cloned().unwrap_or_default();
    item.insert("stackOrder".into(), Value::Null);
    Value::Object(item)
}

fn valid_item(raw: &Value, require_type: bool) -> bool {
    raw.get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty())
        && raw.get("content").is_some_and(Value::is_string)
        && (!require_type || raw.get("type").is_some_and(Value::is_string))
}

fn simulate_import(local: &[Value], incoming: &[Value]) -> Merge {
    let mut result = Merge {
        items: local
            .iter()
            .enumerate()
            .map(|(index, raw)| {
                let mut item = reset_stack(raw);
                if item["id"].is_null() {
                    item["id"] = json!(format!("loc-{index}"));
                }
                item
            })
            .collect(),
        imported: 0,
        skipped: 0,
    };
    for raw in incoming {
        if !valid_item(raw, false)
            || result
                .items
                .iter()
                .any(|item| item_key(item) == item_key(raw) || item.get("id") == raw.get("id"))
        {
            result.skipped += 1;
        } else {
            result.items.push(reset_stack(raw));
            result.imported += 1;
        }
    }
    result
}

fn simulate_sync(local: &[Value], remote: &[Value]) -> Merge {
    let mut result = Merge {
        items: local.iter().map(reset_stack).collect(),
        imported: 0,
        skipped: 0,
    };
    for row in remote {
        let item = row.get("item").filter(|item| truthy(item)).unwrap_or(row);
        if !valid_item(item, true)
            || result
                .items
                .iter()
                .any(|old| item_key(old) == item_key(item) || old.get("id") == item.get("id"))
        {
            result.skipped += 1;
        } else {
            result.items.push(reset_stack(item));
            result.imported += 1;
        }
    }
    if result.imported > 0 {
        result.items.sort_by(|a, b| {
            b["createdAt"]
                .as_f64()
                .unwrap_or(0.0)
                .partial_cmp(&a["createdAt"].as_f64().unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    display_field(a, "id")
                        .encode_utf16()
                        .cmp(display_field(b, "id").encode_utf16())
                })
        });
    }
    result
}

fn parse_hook(line: &str) -> Value {
    let Some((event, command)) = line.split_once('=') else {
        return Value::Null;
    };
    let (event, command) = (trim(event), trim(command));
    if matches!(event, "copy" | "paste") && !command.is_empty() {
        json!({"event":event,"command":command})
    } else {
        Value::Null
    }
}

fn record_accel(event: &Value) -> Result<Value> {
    let code = string(event, "code")?;
    if code == "Escape" {
        return Ok(Value::Null);
    }
    let key = if let Some(letter) = code
        .strip_prefix("Key")
        .filter(|letter| letter.len() == 1 && letter.bytes().all(|byte| byte.is_ascii_uppercase()))
    {
        Some(letter)
    } else if let Some(digit) = code
        .strip_prefix("Digit")
        .filter(|digit| digit.len() == 1 && digit.bytes().all(|byte| byte.is_ascii_digit()))
    {
        Some(digit)
    } else if code.strip_prefix('F').is_some_and(|number| {
        (1..=2).contains(&number.len())
            && number.bytes().all(|byte| byte.is_ascii_digit())
            && number
                .parse::<u8>()
                .is_ok_and(|number| (1..=24).contains(&number))
    }) {
        Some(code)
    } else {
        match code {
            "ArrowUp" => Some("Up"),
            "ArrowDown" => Some("Down"),
            "ArrowLeft" => Some("Left"),
            "ArrowRight" => Some("Right"),
            "Space" | "Enter" | "Tab" | "Backspace" | "Delete" | "Comma" | "Period" | "Slash"
            | "Backslash" | "Minus" | "Equal" | "Semicolon" | "Quote" | "Backquote"
            | "BracketLeft" | "BracketRight" => Some(code),
            _ => None,
        }
    };
    let Some(key) = key else {
        return Ok(Value::Null);
    };
    let mut parts = Vec::new();
    if truthy(&event["ctrl"]) || truthy(&event["meta"]) {
        parts.push("CommandOrControl");
    }
    if truthy(&event["alt"]) {
        parts.push("Alt");
    }
    if truthy(&event["shift"]) {
        parts.push("Shift");
    }
    parts.push(key);
    Ok(json!(parts.join("+")))
}

fn pretty_accel(accelerator: &str) -> String {
    accelerator
        .split('+')
        .map(|part| match part {
            "CommandOrControl" => "⌘/Ctrl",
            "Command" => "⌘",
            "Control" => "Ctrl",
            "Shift" => "⇧",
            "Alt" => "⌥",
            _ => part,
        })
        .collect::<Vec<_>>()
        .join("+")
}

fn desktop_entry(executable: &str) -> String {
    format!("[Desktop Entry]\nType=Application\nName=ClipDeck\nComment=Clipboard manager\nExec=\"{executable}\"\nX-GNOME-Autostart-enabled=true\nHidden=false\nNoDisplay=false\n")
}

fn contents(items: &[Value]) -> Value {
    Value::Array(items.iter().map(|item| item["content"].clone()).collect())
}
fn stacked_count(items: &[Value]) -> usize {
    items
        .iter()
        .filter(|item| !item["stackOrder"].is_null())
        .count()
}
fn label(group: &str, case: &Value, check: &str) -> String {
    format!("{group}/{}: {check}", display_field(case, "id"))
}

fn validate_storage_policy(policy: &Value, checks: &mut Checks) -> Result<()> {
    checks.check(policy["nativeFile"] == json!("clipdeck.sqlite3"), "storage-policy: native SQLite file");
    checks.check(policy["loadLegacyNativeFile"] == json!(false), "storage-policy: legacy native JSON is ignored");
    let integer = |value: &Value, key: &str| value[key].as_u64().ok_or_else(|| format!("storage-policy: expected unsigned {key}"));
    let page_size = integer(policy, "pageSize")?;
    for case in array(policy, "cases")? {
        let count = integer(case, "itemCount")?;
        let offset = integer(case, "offset")?;
        let limit = integer(case, "requestedLimit")?.min(page_size);
        let length = count.saturating_sub(offset).min(limit);
        checks.equal(json!(length), &case["expected"]["count"], label("storage-policy", case, "page count"));
        checks.equal(if length == 0 { Value::Null } else { json!(offset) },
            &case["expected"]["firstIndex"], label("storage-policy", case, "first index"));
        checks.equal(if length == 0 { Value::Null } else { json!(offset + length - 1) },
            &case["expected"]["lastIndex"], label("storage-policy", case, "last index"));
    }
    for case in array(policy, "summaryCases")? {
        let length = string(case, "text")?.chars().count() as u64 * integer(case, "repeat")?;
        for (limit, expected) in [("contentSummaryChars", "contentChars"), ("titleSummaryChars", "titleChars"), ("previewSummaryChars", "previewChars")] {
            checks.equal(json!(length.min(integer(policy, limit)?)), &case["expected"][expected],
                label("storage-policy", case, expected));
        }
    }
    Ok(())
}

fn validate_encryption_policy(policy: &Value, checks: &mut Checks) -> Result<()> {
    for (field, expected) in [
        ("firstChoiceRequired", json!(true)),
        ("keyStorage", json!("os-keyring")),
        ("plaintextFallbackOnKeyFailure", json!(false)),
        ("nativeImages", json!({"storage":"external-files","encryptedWithProfile":true})),
        ("encryptedSyncAllowed", json!(false)),
        ("exportEncrypted", json!(false)),
    ] {
        checks.equal(policy[field].clone(), &expected, format!("encryption-policy: {field}"));
    }
    for case in array(policy, "cases")? {
        let configured = case["configured"].as_bool().ok_or("encryption-policy: expected configured boolean")?;
        let encrypted = case["encrypted"].as_bool().ok_or("encryption-policy: expected encrypted boolean")?;
        let action = if !configured {
            "prompt"
        } else if !encrypted {
            "open-plaintext"
        } else if string(case, "keyStatus")? == "available" {
            "open-encrypted"
        } else {
            "error"
        };
        checks.equal(json!(action), &case["expected"]["action"], label("encryption-policy", case, "action"));
        checks.equal(json!(matches!(action, "open-plaintext" | "open-encrypted")),
            &case["expected"]["mayStartWatching"], label("encryption-policy", case, "watching gate"));
    }
    Ok(())
}

fn validate(root: &Path) -> Result<Checks> {
    let mut checks = Checks::default();
    let mut fixtures = BTreeMap::new();
    let directory = root.join("fixtures");
    let entries = std::fs::read_dir(&directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?;
    let mut file_count = 0;
    for entry in entries {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or("fixture filename is not UTF-8")?;
        file_count += 1;
        fixtures.insert(
            name.to_owned(),
            load_json(root, &format!("fixtures/{name}.json"))?,
        );
    }
    checks.check(fixtures.len() == file_count, "fixture names unique");
    for (name, fixture) in &fixtures {
        let cases = fixture
            .get("cases")
            .filter(|value| !value.is_null())
            .or_else(|| fixture.get("recording").filter(|value| !value.is_null()))
            .or_else(|| fixture.get("parsing").filter(|value| !value.is_null()));
        let empty = Vec::new();
        let cases = match cases {
            Some(cases) => cases
                .as_array()
                .ok_or_else(|| format!("{name}: expected case array"))?,
            None => &empty,
        };
        let ids: BTreeSet<_> = cases
            .iter()
            .map(|case| case.get("id").map(Value::to_string))
            .collect();
        checks.check(ids.len() == cases.len(), format!("{name}: case ids unique"));
    }
    let fixture = |name: &str| {
        fixtures
            .get(name)
            .ok_or_else(|| format!("missing fixture {name}"))
    };
    validate_storage_policy(fixture("storage-policy")?, &mut checks)?;
    validate_encryption_policy(fixture("encryption-policy")?, &mut checks)?;
    for case in array(fixture("classify-text")?, "cases")? {
        checks.equal(
            json!(classify(string(case, "raw")?)),
            &case["expected"],
            label("classify-text", case, "classification"),
        );
    }
    for case in array(fixture("preview-title")?, "cases")? {
        let dimensions = case
            .get("dims")
            .filter(|value| !value.is_null())
            .cloned()
            .unwrap_or_else(|| json!({"width":1,"height":1}));
        let (title, preview) = derive(string(case, "type")?, string(case, "content")?, &dimensions);
        checks.equal(
            json!(title),
            &case["expected"]["title"],
            label("preview-title", case, "title"),
        );
        checks.equal(
            json!(preview),
            &case["expected"]["preview"],
            label("preview-title", case, "preview"),
        );
    }
    let store = fixture("store-rules")?;
    for case in array(store, "cases")? {
        let items = simulate_store(case)?;
        checks.equal(
            contents(&items),
            &case["expected"]["contents"],
            label("store-rules", case, "contents"),
        );
        if let Some(expected) = case["expected"].get("count") {
            checks.equal(
                json!(items.len()),
                expected,
                label("store-rules", case, "count"),
            );
        }
        if let Some(expected) = case["expected"].get("stackedCount") {
            checks.equal(
                json!(stacked_count(&items)),
                expected,
                label("store-rules", case, "stackedCount"),
            );
        }
    }
    for case in array(store, "importCases")? {
        let result = simulate_import(array(case, "local")?, array(case, "import")?);
        checks.equal(
            json!(result.imported),
            &case["expected"]["imported"],
            label("store-rules", case, "imported"),
        );
        checks.equal(
            json!(result.skipped),
            &case["expected"]["skipped"],
            label("store-rules", case, "skipped"),
        );
        checks.equal(
            json!(result.items.len()),
            &case["expected"]["count"],
            label("store-rules", case, "count"),
        );
        if let Some(expected) = case["expected"].get("stackedCount") {
            checks.equal(
                json!(stacked_count(&result.items)),
                expected,
                label("store-rules", case, "stackedCount"),
            );
        }
    }
    for case in array(fixture("sync-merge")?, "cases")? {
        let result = simulate_sync(array(case, "local")?, array(case, "remote")?);
        if truthy(&case["mergeTwice"]) {
            let second = simulate_sync(&result.items, &[]);
            checks.equal(
                json!(result.imported),
                &case["expected"]["firstImported"],
                label("sync-merge", case, "firstImported"),
            );
            checks.equal(
                json!(second.imported),
                &case["expected"]["secondImported"],
                label("sync-merge", case, "secondImported"),
            );
        } else {
            checks.equal(
                json!(result.imported),
                &case["expected"]["imported"],
                label("sync-merge", case, "imported"),
            );
        }
        if let Some(expected) = case["expected"].get("skipped") {
            checks.equal(
                json!(result.skipped),
                expected,
                label("sync-merge", case, "skipped"),
            );
        }
        checks.equal(
            contents(&result.items),
            &case["expected"]["contents"],
            label("sync-merge", case, "contents"),
        );
        if let Some(expected) = case["expected"].get("stackedCount") {
            checks.equal(
                json!(stacked_count(&result.items)),
                expected,
                label("sync-merge", case, "stackedCount"),
            );
        }
    }
    for case in array(fixture("hooks")?, "parsing")? {
        checks.equal(
            parse_hook(string(case, "line")?),
            &case["expected"],
            label("hooks", case, "parse"),
        );
    }
    for case in array(fixture("accelerators")?, "recording")? {
        checks.equal(
            record_accel(&case["event"])?,
            &case["expected"],
            label("accelerators", case, "record"),
        );
    }
    for case in array(fixture("accelerators")?, "display")? {
        checks.equal(
            json!(pretty_accel(string(case, "accel")?)),
            &case["pretty"],
            label("accelerators", case, "display"),
        );
    }
    for case in array(fixture("autostart")?, "cases")? {
        if truthy(&case["expectedFile"]) {
            let executable = case
                .get("appImage")
                .filter(|value| !value.is_null())
                .or_else(|| case.get("execPath"))
                .and_then(Value::as_str)
                .ok_or("expected executable path string")?;
            checks.equal(
                json!(desktop_entry(executable)),
                &case["expectedFile"]["content"],
                label("autostart", case, "desktop entry"),
            );
        } else {
            checks.check(
                case.get("expectedFile") == Some(&Value::Null),
                label("autostart", case, "expected removal"),
            );
        }
    }
    let schema = load_json(root, "schema/clipdeck-store.schema.json")?;
    let sample = load_json(root, "examples/sample-store.json")?;
    checks.check(
        required_fields(&schema, &sample)?,
        "sample-store: top-level required fields present",
    );
    for item in array(&sample, "items")? {
        checks.check(
            required_fields(&schema["$defs"]["clipItem"], item)?,
            format!(
                "sample-store: item {} required fields",
                display_field(item, "id")
            ),
        );
    }
    checks.check(
        required_fields(&schema["$defs"]["settings"], &sample["settings"])?,
        "sample-store: settings required fields present",
    );
    let export_schema = load_json(root, "schema/clipdeck-export.schema.json")?;
    let export = load_json(root, "examples/sample-export.json")?;
    checks.check(export["version"] == json!(1) && required_fields(&export_schema, &export)?,
        "sample-export: version and envelope fields");
    for item in array(&export, "items")? {
        checks.check(required_fields(&schema["$defs"]["clipItem"], item)?,
            format!("sample-export: item {} required fields", display_field(item, "id")));
    }
    for board in array(&export, "pinboards")? {
        checks.check(required_fields(&schema["$defs"]["pinboard"], board)?,
            format!("sample-export: board {} required fields", display_field(board, "id")));
    }
    let security_schema = load_json(root, "schema/profile-security.schema.json")?;
    let security = load_json(root, "examples/sample-profile-security.json")?;
    checks.check(required_fields(&security_schema, &security)?, "sample-profile-security: required mode fields");
    let schema = load_json(root, "schema/sync-line.schema.json")?;
    let path = root.join("examples/sample-sync.jsonl");
    let lines =
        std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    for (index, line) in trim(&lines).split('\n').enumerate() {
        let row: Value = serde_json::from_str(line)
            .map_err(|error| format!("{} line {}: {error}", path.display(), index + 1))?;
        checks.check(
            row["v"] == json!(1) && required_fields(&schema, &row)?,
            format!("sample-sync line {index}: envelope"),
        );
        checks.check(
            required_fields(&schema["properties"]["item"], &row["item"])?,
            format!("sample-sync line {index}: item fields"),
        );
    }
    let defaults = fixture("settings-defaults")?["defaults"]
        .as_object()
        .ok_or("expected defaults object")?;
    for (key, value) in defaults {
        if let Some(actual) = sample["settings"].get(key) {
            checks.equal(
                actual.clone(),
                value,
                format!("sample-store settings.{key} matches defaults"),
            );
        }
    }
    Ok(checks)
}

fn required_fields(schema: &Value, object: &Value) -> Result<bool> {
    let fields = array(schema, "required")?;
    let object = object
        .as_object()
        .ok_or("expected object for schema required fields")?;
    for field in fields {
        if !object.contains_key(
            field
                .as_str()
                .ok_or("schema required field must be a string")?,
        ) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn print_target_dir() -> Result<()> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../apps/linux/Cargo.toml");
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--locked",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(manifest)
        .output()
        .map_err(|error| format!("cargo metadata: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("cargo metadata JSON: {error}"))?;
    println!("{}", string(&metadata, "target_directory")?);
    Ok(())
}

fn run() -> Result<bool> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let root = match args.as_slice() {
        [] => PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        [flag] if flag == "--print-target-dir" => {
            print_target_dir()?;
            return Ok(true);
        }
        [flag] if flag == "--help" || flag == "-h" => {
            println!("Usage: clipdeck-spec [--spec-dir PATH | --print-target-dir]");
            return Ok(true);
        }
        [flag, path] if flag == "--spec-dir" => PathBuf::from(path),
        _ => return Err("Usage: clipdeck-spec [--spec-dir PATH | --print-target-dir]".into()),
    };
    let checks = validate(&root)?;
    println!("\n{} checks, {} failures", checks.count, checks.failures);
    Ok(checks.failures == 0)
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("ERROR: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_truncation_and_ecma_whitespace() {
        assert_eq!(truncate("🎉🎉", 1), "🎉…");
        assert_eq!(classify("\u{feff}#abc\u{feff}"), "color");
        assert_eq!(classify("\u{0085}#abc\u{0085}"), "text");
        assert_eq!(classify("http://"), "text");
    }

    #[test]
    fn urls_keep_whatwg_hostname_semantics() {
        for (content, expected) in [
            ("https://USER:PASS@WWW.EXAMPLE.COM:443/x", "example.com"),
            ("http://[::1]:8080/x", "[::1]"),
            ("mailto:a@b", ""),
            ("https://bücher.example", "xn--bcher-kva.example"),
            ("https://", "https://"),
        ] {
            assert_eq!(derive("link", content, &Value::Null).0, expected);
        }
    }

    #[test]
    fn oracle_keeps_import_and_sync_validation_distinct() {
        let item = json!({"id":"a","content":"text"});
        assert_eq!(simulate_import(&[], &[item.clone()]).imported, 1);
        assert_eq!(simulate_sync(&[], &[item]).skipped, 1);
        let local = [
            json!({"id":"b","type":"text","content":"b","createdAt":2}),
            json!({"id":"a","type":"text","content":"a","createdAt":1}),
        ];
        assert_eq!(
            contents(&simulate_sync(&local, &[]).items),
            json!(["b", "a"])
        );
    }

    #[test]
    fn null_reorder_leaves_capture_order_unchanged() {
        let case = json!({"maxItems":10,"adds":[{"type":"text","content":"first"},{"type":"text","content":"second"}],"reorder":null});
        assert_eq!(
            contents(&simulate_store(&case).unwrap()),
            json!(["second", "first"])
        );
    }

    struct FixtureTree(PathBuf);

    impl FixtureTree {
        fn copy() -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "clipdeck-oracle-test-{}-{unique}",
                std::process::id()
            ));
            let source = Path::new(env!("CARGO_MANIFEST_DIR"));
            for directory in ["fixtures", "schema", "examples"] {
                std::fs::create_dir_all(root.join(directory)).unwrap();
                for entry in std::fs::read_dir(source.join(directory)).unwrap() {
                    let path = entry.unwrap().path();
                    if path.is_file() {
                        std::fs::copy(&path, root.join(directory).join(path.file_name().unwrap()))
                            .unwrap();
                    }
                }
            }
            Self(root)
        }
    }

    impl Drop for FixtureTree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn corrupted_expectation_is_reported_as_a_failure() {
        let tree = FixtureTree::copy();
        let mut fixture = load_json(&tree.0, "fixtures/classify-text.json").unwrap();
        fixture["cases"][0]["expected"] = json!("deliberately-invalid");
        std::fs::write(
            tree.0.join("fixtures/classify-text.json"),
            fixture.to_string(),
        )
        .unwrap();
        let checks = validate(&tree.0).unwrap();
        assert_eq!(checks.count, 244);
        assert_eq!(checks.failures, 1);
    }

    #[test]
    fn malformed_json_is_an_actionable_error() {
        let tree = FixtureTree::copy();
        std::fs::write(tree.0.join("fixtures/classify-text.json"), "{").unwrap();
        let error = validate(&tree.0).unwrap_err();
        assert!(error.contains("classify-text.json"));
        assert!(error.contains("line 1"));
    }

    #[test]
    fn encryption_policy_requires_explicit_choice_and_no_key_fallback() {
        let policy = load_json(Path::new(env!("CARGO_MANIFEST_DIR")), "fixtures/encryption-policy.json").unwrap();
        assert_eq!(policy["keyStorage"], json!("os-keyring"));
        assert_eq!(policy["plaintextFallbackOnKeyFailure"], json!(false));
    }

    #[test]
    fn sqlite_storage_policy_fixture_is_present() {
        let policy = load_json(Path::new(env!("CARGO_MANIFEST_DIR")), "fixtures/storage-policy.json").unwrap();
        assert_eq!(policy["nativeFile"], json!("clipdeck.sqlite3"));
        assert_eq!(policy["loadLegacyNativeFile"], json!(false));
    }

    #[test]
    fn repository_fixture_tree_retains_all_checks() {
        let result = validate(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        assert_eq!(result.count, 244);
        assert_eq!(result.failures, 0);
    }
}
