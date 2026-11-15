//! Launch-at-login per spec/behavior.md §11. Linux writes a byte-exact
//! ~/.config/autostart entry; other platforms delegate to OS APIs in the app.

use std::path::{Path, PathBuf};

/// Byte-exact desktop entry (ends with a trailing newline).
pub fn render_desktop_entry(exec_path: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=ClipDeck\nComment=Clipboard manager\nExec=\"{exec_path}\"\nX-GNOME-Autostart-enabled=true\nHidden=false\nNoDisplay=false\n"
    )
}

pub fn autostart_file(home: &Path) -> PathBuf {
    home.join(".config").join("autostart").join("clipdeck.desktop")
}

/// Enable/disable the Linux autostart entry. `exec_path` is the AppImage path
/// when running from an AppImage, otherwise the native binary path.
pub fn apply_login_item_linux(enable: bool, exec_path: &str, home: &Path) -> std::io::Result<()> {
    let file = autostart_file(home);
    if enable {
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&file, render_desktop_entry(exec_path))
    } else {
        match std::fs::remove_file(&file) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}
