//! Text classification per spec/behavior.md §3. The indentation signal is
//! tested against raw text; all other signals against trimmed text.

use crate::types::ItemType;

const KEYWORD_SIGNALS: [&str; 10] = [
    "const ", "let ", "var ", "function ", "def ", "class ", "import ", "#include", "</", "/>",
];

pub fn classify(raw: &str) -> ItemType {
    let t = raw.trim();
    if is_hex_color(t) || t.starts_with("rgb(") || t.starts_with("rgba(") {
        return ItemType::Color;
    }
    if is_link(t) {
        return ItemType::Link;
    }
    let mut score = 0u8;
    if t.chars().any(|c| matches!(c, ';' | '{' | '}' | '(' | ')' | '[' | ']')) {
        score += 1;
    }
    if KEYWORD_SIGNALS.iter().any(|k| t.contains(k)) {
        score += 1;
    }
    if t.contains('\n') && (raw.contains('\t') || raw.contains("  ")) {
        score += 1;
    }
    if score >= 2 {
        ItemType::Code
    } else {
        ItemType::Text
    }
}

/// `^#[0-9a-fA-F]{3,8}$` on trimmed text.
fn is_hex_color(t: &str) -> bool {
    let Some(rest) = t.strip_prefix('#') else { return false };
    let n = rest.chars().count();
    (3..=8).contains(&n) && rest.chars().all(|c| c.is_ascii_hexdigit())
}

/// `^(https?:\/\/|www\.)\S+$` on trimmed text.
fn is_link(t: &str) -> bool {
    let rest = if let Some(r) = t.strip_prefix("https://") {
        r
    } else if let Some(r) = t.strip_prefix("http://") {
        r
    } else if let Some(r) = t.strip_prefix("www.") {
        r
    } else {
        return false;
    };
    !rest.is_empty() && !rest.chars().any(char::is_whitespace)
}
