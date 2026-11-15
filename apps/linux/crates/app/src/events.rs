//! `AppEvent` — the single point of truth for every cross-thread signal and
//! state mutation. Producers (clipboard worker, tray, hotkey thread, control
//! socket, UI handlers) only ever SEND these; exactly one dispatcher applies
//! them on the main thread. This is the whole inter-component contract.

use clipdeck_core::ClipItem;

/// Where a clipboard write should lead once the worker reports it done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritePurpose {
    Copy,
    /// Stack clipboard write (no synthetic typing).
    StackPaste,
    /// Tray "recent item" click.
    TrayCopy,
}

/// Payload for the clipboard worker (the sole owner of arboard).
#[derive(Debug, Clone)]
pub enum WritePayload {
    Text(String),
    Image { width: usize, height: usize, rgba: Vec<u8> },
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    /// Immutable result from the bounded read-only page worker.
    PageLoaded { token: u64, page: Result<clipdeck_core::ItemPage, String> },

    /* producers -> main */
    /// Clipboard worker captured a change (raw; main applies skip rules).
    Captured { item: ClipItem, source_app: Option<String>, image: Option<Vec<u8>> },
    /// Clipboard worker finished a write.
    Written { purpose: WritePurpose },

    /// Request an explicit at-rest encryption choice/change.
    ConfigureEncryption { enabled: bool },
    /// System keyring preparation completed outside the UI thread.
    EncryptionPrepared { enabled: bool, result: Result<(), String> },

    /* window / lifecycle */
    Show,
    Hide,
    Toggle,
    OpenSettings,
    OpenKeyboardSettings,
    CopyShortcutCommand,
    Quit,

    /* state mutations (all funneled through the one dispatcher) */
    ToggleWatch,
    Copy { id: String, purpose: WritePurpose },
    Activate { id: String, plain: bool },
    StackPush { id: String },
    StackPop,
    StackClear,
    StackUnstack { id: String },
    Delete { id: String },
    Pin { id: String, pinned: bool },
    NewPinboard,
    DeletePinboard { id: String },
    RenamePinboard { id: String, name: String },
    MoveToPinboard { id: String, pinboard_id: Option<String> },
    SaveEdit { id: String, content: String },
    SetShortcut { accel: String },
    ToggleSetting { key: &'static str },
}
