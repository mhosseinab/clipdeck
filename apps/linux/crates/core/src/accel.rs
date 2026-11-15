//! Global-shortcut accelerator recording/display per spec/behavior.md §10.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyPress {
    pub ctrl: bool,
    pub meta: bool,
    pub alt: bool,
    pub shift: bool,
    /// Physical key code in the DOM `KeyboardEvent.code` vocabulary.
    pub code: String,
}

/// Record one key press into a canonical Electron-style accelerator.
/// Modifier-only presses and unsupported keys yield `None`; `Escape` cancels
/// recording (yields `None`).
pub fn record_accel(ev: &KeyPress) -> Option<String> {
    if ev.code == "Escape" {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    if ev.ctrl || ev.meta {
        parts.push("CommandOrControl");
    }
    if ev.alt {
        parts.push("Alt");
    }
    if ev.shift {
        parts.push("Shift");
    }
    let key = key_from_code(&ev.code)?;
    parts.push(key);
    Some(parts.join("+"))
}

fn key_from_code(code: &str) -> Option<&'static str> {
    if let Some(rest) = code.strip_prefix("Key") {
        if rest.len() == 1 {
            let c = rest.chars().next().unwrap();
            if c.is_ascii_uppercase() {
                return Some(match c {
                    'A' => "A", 'B' => "B", 'C' => "C", 'D' => "D", 'E' => "E", 'F' => "F",
                    'G' => "G", 'H' => "H", 'I' => "I", 'J' => "J", 'K' => "K", 'L' => "L",
                    'M' => "M", 'N' => "N", 'O' => "O", 'P' => "P", 'Q' => "Q", 'R' => "R",
                    'S' => "S", 'T' => "T", 'U' => "U", 'V' => "V", 'W' => "W", 'X' => "X",
                    'Y' => "Y", _ => "Z",
                });
            }
        }
        return None;
    }
    if let Some(rest) = code.strip_prefix("Digit") {
        if rest.len() == 1 && rest.as_bytes()[0].is_ascii_digit() {
            return Some(match rest.as_bytes()[0] {
                b'0' => "0", b'1' => "1", b'2' => "2", b'3' => "3", b'4' => "4",
                b'5' => "5", b'6' => "6", b'7' => "7", b'8' => "8", _ => "9",
            });
        }
        return None;
    }
    if let Some(rest) = code.strip_prefix("F") {
        if !rest.is_empty() && rest.len() <= 2 && rest.bytes().all(|b| b.is_ascii_digit()) {
            let n: u32 = rest.parse().ok()?;
            return match n {
                1 => Some("F1"), 2 => Some("F2"), 3 => Some("F3"), 4 => Some("F4"),
                5 => Some("F5"), 6 => Some("F6"), 7 => Some("F7"), 8 => Some("F8"),
                9 => Some("F9"), 10 => Some("F10"), 11 => Some("F11"), 12 => Some("F12"),
                13 => Some("F13"), 14 => Some("F14"), 15 => Some("F15"), 16 => Some("F16"),
                17 => Some("F17"), 18 => Some("F18"), 19 => Some("F19"), 20 => Some("F20"),
                21 => Some("F21"), 22 => Some("F22"), 23 => Some("F23"), 24 => Some("F24"),
                _ => None,
            };
        }
        return None;
    }
    Some(match code {
        "Space" => "Space", "Enter" => "Enter", "Escape" => "Escape", "Tab" => "Tab",
        "Backspace" => "Backspace", "Delete" => "Delete",
        "ArrowUp" => "Up", "ArrowDown" => "Down", "ArrowLeft" => "Left", "ArrowRight" => "Right",
        "Comma" => "Comma", "Period" => "Period", "Slash" => "Slash", "Backslash" => "Backslash",
        "Minus" => "Minus", "Equal" => "Equal", "Semicolon" => "Semicolon", "Quote" => "Quote",
        "Backquote" => "Backquote", "BracketLeft" => "BracketLeft", "BracketRight" => "BracketRight",
        _ => return None,
    })
}

/// Human display form: `CommandOrControl+Shift+V` → `⌘/Ctrl+⇧+V`.
pub fn pretty_accel(accel: &str) -> String {
    accel
        .split('+')
        .map(|p| match p {
            "CommandOrControl" => "\u{2318}/Ctrl",
            "Command" => "\u{2318}",
            "Control" => "Ctrl",
            "Shift" => "\u{21e7}",
            "Alt" => "\u{2325}",
            other => other,
        })
        .collect::<Vec<_>>()
        .join("+")
}
