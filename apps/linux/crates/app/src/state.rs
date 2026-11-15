//! `AppState` — the single owner of all mutable application state.
//! Pure main-thread object: no locks, no I/O inside decision logic. Every
//! mutation is a named method (single point of truth for state transitions);
//! side-effecting I/O (clipboard, hooks, sounds) is handed back to the caller
//! as data so the caller decides which worker performs it.

use clipdeck_core::{
    run_hooks, ClipItem, ItemType, Settings, StackDir, Store, SyncStore,
};
use std::path::{Path, PathBuf};

use crate::events::{WritePayload, WritePurpose};

/// Side effects a mutation asks for; the dispatcher routes them to workers.
#[derive(Debug)]
pub struct Effects {
    pub writes: Vec<(WritePayload, WritePurpose)>,
    pub hooks: Vec<(&'static str, String)>,
    pub sound: Option<&'static str>,
    pub hidden: bool,
    pub open_keyboard_settings: bool,
    pub watching: Option<bool>,
    pub configure_encryption: Option<bool>,
    pub restart_for_encryption: bool,
    pub capture_completed: bool,
}

impl Effects {
    pub fn none() -> Self {
        Effects { writes: vec![], hooks: vec![], sound: None, hidden: false, open_keyboard_settings: false, watching: None, configure_encryption: None, restart_for_encryption: false, capture_completed: false }
    }
}

pub struct AppState {
    pub store: Store,
    sync: SyncStore,
    pub generation: u64,
    pub privacy: crate::security::PrivacyConfig,
    profile: PathBuf,
}

impl AppState {
    #[cfg(test)]
    pub fn open(user_data: &Path) -> AppState {
        Self::try_open(user_data).unwrap_or_else(|error| panic!("{error}"))
    }

    pub fn try_open(user_data: &Path) -> Result<AppState, String> { Self::open_profile(user_data, false) }

    pub fn try_open_for_daemon(user_data: &Path) -> Result<AppState, String> { Self::open_profile(user_data, true) }

    fn open_profile(user_data: &Path, apply_pending: bool) -> Result<AppState, String> {
        let (store, privacy) = crate::security::open_store(user_data, apply_pending)?;
        let sync = SyncStore::new(if privacy.encrypted { "" } else { &store.settings.sync_directory });
        Ok(AppState { store, sync, generation: 0, privacy, profile:user_data.to_owned() })
    }

    pub fn profile_path(&self) -> &Path { &self.profile }

    pub fn settings(&self) -> &Settings {
        &self.store.settings
    }

    fn bump(&mut self) {
        self.generation += 1;
    }

    /* ---------- capture pipeline (spec §2) ---------- */

    /// Apply skip rules + dedupe + record. Returns the effect bundle.
    #[cfg(test)]
    pub fn record_captured(&mut self, item: ClipItem, source_app: Option<String>) -> Effects {
        self.record_captured_image(item, source_app, None)
    }

    pub fn record_captured_image(&mut self, item: ClipItem, source_app: Option<String>, image: Option<Vec<u8>>) -> Effects {
        if !self.privacy.configured || !self.store.settings.watch_enabled {
            return Effects::none();
        }
        let s = self.store.settings.clone();
        if let Some(app) = source_app.as_ref().map(|a| a.to_lowercase()) {
            let blocked = s
                .excluded_apps
                .iter()
                .filter(|e| !e.is_empty())
                .any(|e| app.contains(&e.to_lowercase()));
            let confidential = s.ignore_confidential
                && ["keychain", "password", "1password", "bitwarden", "keepass", "lastpass"]
                    .iter()
                    .any(|p| app.contains(p));
            if blocked || confidential {
                return Effects::none();
            }
        }
        let mut item = item;
        item.source_app = source_app;
        let saved = self.store.add_item_with_image(item, image);
        self.sync.append(&saved);
        self.bump();
        let mut fx = Effects::none();
        if self.store.settings.sound_copy {
            fx.sound = Some("copy");
        }
        fx
    }

    /* ---------- clipboard intents (spec §7) ---------- */

    fn payload_for(&self, item: ClipItem, plain: bool) -> WritePayload {
        if !plain && item.item_type == ItemType::Image {
            if let Some(name) = item.image_file.as_ref() {
                if let Some(png) = self.store.image_bytes(name) {
                    if let Some((w, h, rgba)) = crate::worker::decode_png(&png) {
                        return WritePayload::Image { width: w, height: h, rgba };
                    }
                }
            }
        }
        WritePayload::Text(item.content)
    }

    /// Linux overlay activation copies and closes; it never injects input.
    pub fn activate(&mut self, id: &str, plain: bool) -> Effects {
        let mut effects = self.copy_item(id, WritePurpose::Copy, plain);
        effects.hidden = !effects.writes.is_empty();
        effects
    }

    pub fn copy(&mut self, id: &str, purpose: WritePurpose) -> Effects {
        self.copy_item(id, purpose, false)
    }

    fn copy_item(&mut self, id: &str, purpose: WritePurpose, plain: bool) -> Effects {
        let Some(item) = self.store.find(id) else { return Effects::none() };
        let mut fx = Effects::none();
        if self.store.settings.hooks.iter().any(|hook| hook.event == "copy") {
            fx.hooks.push(("copy", item.content.clone()));
        }
        let payload = self.payload_for(item, plain);
        fx.writes.push((payload, purpose));
        if self.store.settings.sound_copy {
            fx.sound = Some("copy");
        }
        fx
    }

    /* ---------- stack (spec §6) ---------- */

    pub fn stack_push(&mut self, id: &str) -> Effects {
        self.store.push_stack(id);
        self.bump();
        Effects::none()
    }

    pub fn stack_pop(&mut self) -> Effects {
        let dir = if self.store.settings.stack_direction == "lifo" { StackDir::Lifo } else { StackDir::Fifo };
        let Some(item) = self.store.peek_stack(dir) else { return Effects::none() };
        let mut fx = Effects::none();
        if self.store.settings.hooks.iter().any(|hook| hook.event == "paste") {
            fx.hooks.push(("paste", item.content.clone()));
        }
        let id = item.id.clone();
        let payload = self.payload_for(item, self.store.settings.paste_as_plain_text);
        self.store.remove(&id);
        self.bump();
        fx.writes.push((payload, WritePurpose::StackPaste));
        fx.hidden = true;
        fx
    }

    pub fn stack_unstack(&mut self, id: &str) -> Effects {
        self.store.unstack(id);
        self.bump();
        Effects::none()
    }

    pub fn stack_clear(&mut self) -> Effects {
        self.store.clear_stack();
        self.bump();
        Effects::none()
    }

    /* ---------- item mutations ---------- */

    pub fn delete(&mut self, id: &str) -> Effects {
        self.store.remove(id);
        self.bump();
        Effects::none()
    }

    pub fn pin(&mut self, id: &str, pinned: bool) -> Effects {
        self.store.set_pinned(id, pinned);
        self.bump();
        Effects::none()
    }

    pub fn add_pinboard(&mut self) -> Effects {
        let name = format!("Pinboard {}", self.store.pinboards.len() + 1);
        self.store.add_pinboard(&name);
        self.bump();
        Effects::none()
    }

    pub fn delete_pinboard(&mut self, id: &str) -> Effects {
        self.store.remove_pinboard(id);
        self.bump();
        Effects::none()
    }

    pub fn rename_pinboard(&mut self, id: &str, name: &str) -> Effects {
        let name = name.trim();
        if name.is_empty() {
            return Effects::none();
        }
        let Some(board) = self.store.pinboards.iter_mut().find(|board| board.id == id) else {
            return Effects::none();
        };
        board.name = name.to_owned();
        self.bump();
        Effects::none()
    }

    pub fn move_to_pinboard(&mut self, id: &str, pinboard_id: Option<&str>) -> Effects {
        if let Some(target) = pinboard_id {
            if !self.store.pinboards.iter().any(|board| board.id == target) {
                return Effects::none();
            }
        }
        if !self.store.contains_id(id) {
            return Effects::none();
        }
        self.store.set_board(id, pinboard_id);
        self.bump();
        Effects::none()
    }

    pub fn save_edit(&mut self, id: &str, content: &str) -> Effects {
        self.store.update_content(id, content);
        self.bump();
        Effects::none()
    }

    pub fn toggle_watch(&mut self) -> Effects {
        self.toggle_setting("watch")
    }

    /// All settings toggles in one place (single point of truth).
    pub fn toggle_setting(&mut self, key: &str) -> Effects {
        {
            let s = &mut self.store.settings;
            match key {
                "watch" => s.watch_enabled = !s.watch_enabled,
                "sound_copy" => s.sound_copy = !s.sound_copy,
                "sound_paste" => s.sound_paste = !s.sound_paste,
                "transient" => s.ignore_transient = !s.ignore_transient,
                "confidential" => s.ignore_confidential = !s.ignore_confidential,
                "link_previews" => s.generate_link_previews = !s.generate_link_previews,
                "plain" => s.paste_as_plain_text = !s.paste_as_plain_text,
                "share" => s.show_in_screen_share = !s.show_in_screen_share,
                "stack_dir" => {
                    s.stack_direction = if s.stack_direction == "fifo" { "lifo".into() } else { "fifo".into() }
                }
                "login" => {
                    s.launch_at_login = !s.launch_at_login;
                    let on = s.launch_at_login;
                    let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
                    clipdeck_core::apply_login_item_linux(on, &exe, &home).ok();
                }
                _ => {}
            }
        }
        self.bump();
        let mut effects = Effects::none();
        if key == "watch" {
            effects.watching = Some(self.store.settings.watch_enabled);
        }
        effects
    }

    /* ---------- data access (read side) ---------- */

    pub fn run_hooks_for(&self, event: &str, content: &str) {
        run_hooks(event, content, &self.store.settings.hooks);
    }

    pub fn save(&mut self) {
        self.store.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_not_recorded_before_the_privacy_choice() {
        let root = std::env::temp_dir().join(format!("clipdeck-choice-{}", clipdeck_core::store::uuid_v4ish()));
        let mut app = AppState::open(&root);
        app.store.settings.watch_enabled = true;
        assert!(!app.privacy.configured);
        let item = ClipItem::from_import(&serde_json::json!({"id":"before-choice","type":"text","content":"not consented"})).unwrap();
        app.record_captured_image(item, None, None);
        assert!(app.store.is_empty(), "first-run choice must precede collection");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn copying_without_hooks_does_not_duplicate_content_for_hooks() {
        let root = std::env::temp_dir().join(format!("clipdeck-copy-{}", clipdeck_core::store::uuid_v4ish()));
        let mut app = AppState::open(&root);
        app.store.settings.hooks.clear();
        let content = "large text beyond any card excerpt ".repeat(1000);
        let item = ClipItem::from_import(&serde_json::json!({
            "id": "full-text", "type": "text", "content": content
        })).unwrap();
        app.store.add_item(item);
        let effects = app.copy("full-text", WritePurpose::Copy);
        assert!(effects.hooks.is_empty(), "unused hook payload duplicates a full clip");
        let WritePayload::Text(copied) = &effects.writes[0].0 else { panic!("text payload expected") };
        assert_eq!(copied, &content, "copy must fetch full text, not its card excerpt");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn paused_capture_preserves_history_and_sync() {
        let root = std::env::temp_dir().join(format!("clipdeck-pause-{}", clipdeck_core::store::uuid_v4ish()));
        let mut app = AppState::open(&root);
        app.privacy.configured = true;
        app.store.settings.watch_enabled = false;
        let item = ClipItem::from_import(&serde_json::json!({
            "id": "paused-secret", "type": "text", "content": "sensitive content"
        })).unwrap();
        let effects = app.record_captured(item.clone(), Some("TestApp".into()));
        assert!(app.store.is_empty(), "paused clipboard content must not enter history");
        assert_eq!(app.generation, 0);
        assert!(effects.writes.is_empty());
        assert!(effects.hooks.is_empty());
        assert!(effects.sound.is_none());
        app.store.settings.watch_enabled = true;
        app.record_captured(item, Some("TestApp".into()));
        assert_eq!(app.store.len(), 1, "capture resumes when enabled");
        std::fs::remove_dir_all(root).unwrap();
    }
}
