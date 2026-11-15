use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const ELLIPSIS: &str = "\u{2026}";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemType {
    Text,
    Link,
    Code,
    Color,
    Image,
    File,
    Html,
}

impl ItemType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ItemType::Text => "text",
            ItemType::Link => "link",
            ItemType::Code => "code",
            ItemType::Color => "color",
            ItemType::Image => "image",
            ItemType::File => "file",
            ItemType::Html => "html",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipItem {
    pub id: String,
    #[serde(rename = "type")]
    pub item_type: ItemType,
    pub content: String,
    pub preview: String,
    pub title: String,
    pub source_app: Option<String>,
    pub created_at: i64,
    pub size_bytes: u64,
    pub pinned: bool,
    pub pinboard_id: Option<String>,
    pub stack_order: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_file: Option<String>,
}

impl ClipItem {
    /// Dedupe key per behavior.md §2.4.
    pub fn dedupe_key(&self) -> String {
        format!("{}|{}", self.item_type.as_str(), self.content)
    }

    /// Trim/keep classification per behavior.md §5.2.
    pub fn is_keep(&self) -> bool {
        self.pinned || self.pinboard_id.is_some() || self.stack_order.is_some()
    }

    pub fn is_dedupe_candidate(&self) -> bool {
        self.stack_order.is_none()
    }

    /// Lenient parser for import (§5.3) and sync rows (§9): requires only
    /// non-empty `id`, string `content` and a valid `type`. Missing derived
    /// fields are filled — `preview`/`title` via derivation, the rest defaulted.
    pub fn from_import(v: &Value) -> Result<ClipItem, String> {
        let get_str = |k: &str| -> Result<String, String> {
            v.get(k)
                .and_then(|x| x.as_str())
                .map(String::from)
                .ok_or_else(|| format!("missing {k}"))
        };
        let id = get_str("id")?;
        if id.is_empty() {
            return Err("empty id".into());
        }
        let content = get_str("content").map_err(|_| "non-string content")?;
        let item_type: ItemType = serde_json::from_value(v.get("type").cloned().ok_or("missing type")?)
            .map_err(|_| "bad type")?;
        let (title, preview) = match v.get("title").and_then(|x| x.as_str()) {
            Some(t) if !t.is_empty() => (t.to_string(), v.get("preview").and_then(|x| x.as_str()).unwrap_or(t).to_string()),
            _ => crate::derive::derive_pair(item_type, &content, None),
        };
        Ok(ClipItem {
            id,
            item_type,
            content,
            preview,
            title,
            source_app: v.get("sourceApp").and_then(|x| x.as_str()).map(String::from),
            created_at: v.get("createdAt").and_then(|x| x.as_i64()).unwrap_or(0),
            size_bytes: v.get("sizeBytes").and_then(|x| x.as_u64()).unwrap_or(0),
            pinned: v.get("pinned").and_then(|x| x.as_bool()).unwrap_or(false),
            pinboard_id: v.get("pinboardId").and_then(|x| x.as_str()).map(String::from),
            stack_order: None,
            image_file: v.get("imageFile").and_then(|x| x.as_str()).map(String::from),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pinboard {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub position: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hook {
    pub event: String,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub watch_enabled: bool,
    pub sound_copy: bool,
    pub sound_paste: bool,
    pub max_items: u32,
    pub shortcut: String,
    pub launch_at_login: bool,
    pub theme: String,
    pub excluded_apps: Vec<String>,
    pub ignore_transient: bool,
    pub ignore_confidential: bool,
    pub generate_link_previews: bool,
    pub paste_as_plain_text: bool,
    pub stack_direction: String,
    pub plain_text_apps: Vec<String>,
    pub sync_directory: String,
    pub show_in_screen_share: bool,
    pub hooks: Vec<Hook>,
    /// Unknown keys read from disk are preserved on write (behavior.md §1.3).
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            watch_enabled: true,
            sound_copy: true,
            sound_paste: true,
            max_items: 5000,
            shortcut: "CommandOrControl+Shift+V".into(),
            launch_at_login: true,
            theme: "dark".into(),
            excluded_apps: vec![
                "com.apple.keychainaccess".into(),
                "com.apple.Passwords".into(),
                "1Password".into(),
                "Bitwarden".into(),
                "KeePass".into(),
            ],
            ignore_transient: true,
            ignore_confidential: true,
            generate_link_previews: true,
            paste_as_plain_text: false,
            stack_direction: "fifo".into(),
            plain_text_apps: vec![],
            sync_directory: String::new(),
            show_in_screen_share: true,
            hooks: vec![],
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreFile {
    pub items: Vec<ClipItem>,
    pub pinboards: Vec<Pinboard>,
    pub settings: Settings,
}
