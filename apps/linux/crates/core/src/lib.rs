//! ClipDeck core: store, classification, derivation, sync, hooks, accelerators.
//! Behavior is defined by `spec/behavior.md`; `tests/fixtures.rs` proves this
//! crate matches the shared behavior fixtures and independent Rust oracle.

pub mod accel;
pub mod autostart;
pub mod classify;
pub mod crypto;
pub mod derive;
pub mod hooks;
pub mod store;
pub mod sync;
pub mod types;

pub use accel::{pretty_accel, record_accel, KeyPress};
pub use autostart::{apply_login_item_linux, autostart_file, render_desktop_entry};
pub use classify::classify;
pub use crypto::EncryptionKey;
pub use derive::{derive_pair, truncate_scalars};
pub use hooks::{parse_hook_line, run_hooks};
pub use store::{ClipFilter, ItemPage, StoreReader, ImportCounts, StackDir, Store, PAGE_SIZE, DISPLAY_CONTENT_CHARS, SQLITE_CACHE_KIB};
pub use sync::{MergeCounts, SyncStore};
pub use types::{ClipItem, Hook, ItemType, Pinboard, Settings, ELLIPSIS};
