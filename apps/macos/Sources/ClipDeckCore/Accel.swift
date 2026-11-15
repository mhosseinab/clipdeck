import Foundation

/// Global-shortcut accelerator recording/display per spec/behavior.md §10.
/// Key codes use the DOM `KeyboardEvent.code` vocabulary for cross-platform
/// fixture compatibility.
public struct KeyPress: Equatable, Sendable {
    public var ctrl: Bool
    public var meta: Bool
    public var alt: Bool
    public var shift: Bool
    public var code: String
    public init(ctrl: Bool, meta: Bool, alt: Bool, shift: Bool, code: String) {
        self.ctrl = ctrl
        self.meta = meta
        self.alt = alt
        self.shift = shift
        self.code = code
    }
}

public enum Accel {
    private static let codeMap: [String: String] = [
        "Space": "Space", "Enter": "Enter", "Escape": "Escape", "Tab": "Tab",
        "Backspace": "Backspace", "Delete": "Delete",
        "ArrowUp": "Up", "ArrowDown": "Down", "ArrowLeft": "Left", "ArrowRight": "Right",
        "Comma": "Comma", "Period": "Period", "Slash": "Slash", "Backslash": "Backslash",
        "Minus": "Minus", "Equal": "Equal", "Semicolon": "Semicolon", "Quote": "Quote",
        "Backquote": "Backquote", "BracketLeft": "BracketLeft", "BracketRight": "BracketRight",
    ]

    private static let fnKeys = Set((1...24).map { "F\($0)" })

    /// Record one key press into a canonical accelerator.
    public static func record(_ ev: KeyPress) -> String? {
        if ev.code == "Escape" { return nil }
        var parts: [String] = []
        if ev.ctrl || ev.meta { parts.append("CommandOrControl") }
        if ev.alt { parts.append("Alt") }
        if ev.shift { parts.append("Shift") }
        guard let key = keyFromCode(ev.code) else { return nil }
        parts.append(key)
        return parts.joined(separator: "+")
    }

    static func keyFromCode(_ code: String) -> String? {
        if code.hasPrefix("Key") {
            let rest = String(code.dropFirst(3))
            guard rest.count == 1, let c = rest.first, ("A"..."Z").contains(String(c)) else { return nil }
            return rest
        }
        if code.hasPrefix("Digit") {
            let rest = String(code.dropFirst(5))
            guard rest.count == 1, "0123456789".contains(rest) else { return nil }
            return rest
        }
        if fnKeys.contains(code) { return code }
        return codeMap[code]
    }

    /// Human display form: `CommandOrControl+Shift+V` → `⌘/Ctrl+⇧+V`.
    public static func pretty(_ accel: String) -> String {
        accel.split(separator: "+").map { part -> String in
            switch part {
            case "CommandOrControl": return "\u{2318}/Ctrl"
            case "Command": return "\u{2318}"
            case "Control": return "Ctrl"
            case "Shift": return "\u{21e7}"
            case "Alt": return "\u{2325}"
            default: return String(part)
            }
        }.joined(separator: "+")
    }
}
