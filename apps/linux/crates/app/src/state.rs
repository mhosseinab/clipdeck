//! Shared daemon state: store + sync + generation counter for UI refresh.

use clipdeck_core::{ItemType, Settings, Store, SyncStore};
use std::path::Path;

pub struct AppState {
    pub store: Store,
    pub sync: SyncStore,
    pub generation: u64,
    pub last_app: Option<String>,
}

impl AppState {
    pub fn open(user_data: &Path) -> AppState {
        let store = Store::open(user_data);
        let sync = SyncStore::new(&store.settings.sync_directory);
        AppState { store, sync, generation: 0, last_app: None }
    }

    pub fn settings(&self) -> &Settings {
        &self.store.settings
    }

    pub fn bump(&mut self) {
        self.generation += 1;
    }

    /// §7 paste: resolve plain-text policy, write clipboard, fire hooks.
    pub fn paste_item(&mut self, id: &str, force_plain: Option<bool>) -> bool {
        let Some(item) = self.store.find(id).cloned() else { return false };
        let s = self.store.settings.clone();
        let rule_plain = self
            .last_app
            .as_ref()
            .map(|a| {
                s.plain_text_apps
                    .iter()
                    .any(|p| !p.is_empty() && a.to_lowercase().contains(&p.to_lowercase()))
            })
            .unwrap_or(false);
        let plain = force_plain.unwrap_or(s.paste_as_plain_text || rule_plain);
        crate::paste::write_item(&self.store, &item, plain);
        clipdeck_core::run_hooks("paste", &item.content, &s.hooks);
        self.bump();
        true
    }

    pub fn copy_item(&mut self, id: &str) -> bool {
        let Some(item) = self.store.find(id).cloned() else { return false };
        crate::paste::write_item(&self.store, &item, false);
        clipdeck_core::run_hooks("copy", &item.content, &self.store.settings.hooks);
        true
    }

    pub fn pop_stack(&mut self) -> bool {
        let dir = if self.store.settings.stack_direction == "lifo" {
            clipdeck_core::StackDir::Lifo
        } else {
            clipdeck_core::StackDir::Fifo
        };
        let Some(item) = self.store.pop_stack(dir) else { return false };
        crate::paste::write_item(&self.store, &item, self.store.settings.paste_as_plain_text);
        clipdeck_core::run_hooks("paste", &item.content, &self.store.settings.hooks);
        self.store.remove(&item.id);
        self.store.save();
        self.bump();
        true
    }

    /// Record a captured item per §2 (already classified and derived upstream).
    pub fn record(
        &mut self,
        item_type: ItemType,
        content: String,
        title: String,
        preview: String,
        size_bytes: u64,
        image_file: Option<String>,
    ) {
        let item = clipdeck_core::ClipItem {
            id: clipdeck_core::store::uuid_v4ish(),
            item_type,
            content,
            preview,
            title,
            source_app: self.last_app.clone(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
            size_bytes,
            pinned: false,
            pinboard_id: None,
            stack_order: None,
            image_file,
        };
        let saved = self.store.add_item(item);
        self.sync.append(&saved);
        if self.store.settings.sound_copy {
            crate::sounds::play("copy");
        }
        self.store.save();
        self.bump();
    }
}
