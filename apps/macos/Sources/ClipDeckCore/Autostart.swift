import Foundation

/// Launch-at-login per spec/behavior.md §11. The macOS app target performs the
/// actual SMAppService registration; this module renders the Linux desktop
/// entry for parity (used by tests and tooling).
public enum Autostart {
    /// Byte-exact desktop entry (ends with a trailing newline).
    public static func renderDesktopEntry(execPath: String) -> String {
        "[Desktop Entry]\nType=Application\nName=ClipDeck\nComment=Clipboard manager\nExec=\"\(execPath)\"\nX-GNOME-Autostart-enabled=true\nHidden=false\nNoDisplay=false\n"
    }

    /// Exec selection: AppImage path when present, otherwise the native binary.
    public static func execPath(binary: String, appImage: String?) -> String {
        appImage ?? binary
    }
}
