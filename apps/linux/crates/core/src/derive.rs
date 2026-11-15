//! Preview/title derivation per spec/behavior.md §4. Truncation counts
//! Unicode scalar values (`char`s in Rust are scalar values) and appends
//! U+2026 only when truncation occurs.

use crate::types::{ItemType, ELLIPSIS};

pub const TITLE_CAP: usize = 60;
pub const PREVIEW_CAP: usize = 120;
pub const LINK_FALLBACK_CAP: usize = 80;

/// Truncate to `cap` Unicode scalars, appending `…` when shortened.
pub fn truncate_scalars(s: &str, cap: usize) -> String {
    if s.chars().count() <= cap {
        return s.to_string();
    }
    let head: String = s.chars().take(cap).collect();
    head + ELLIPSIS
}

/// (title, preview) for a stored item.
pub fn derive_pair(
    item_type: ItemType,
    content: &str,
    image_dims: Option<(u32, u32)>,
) -> (String, String) {
    match item_type {
        ItemType::Link => {
            let host = link_host(content)
                .unwrap_or_else(|| truncate_scalars(content, LINK_FALLBACK_CAP));
            (host.clone(), host)
        }
        ItemType::Color => {
            let t = content.trim().to_string();
            (t.clone(), t)
        }
        ItemType::Image => {
            let (w, h) = image_dims.unwrap_or((0, 0));
            ("Image".into(), format!("{w}\u{d7}{h} image"))
        }
        ItemType::File => {
            let lines: Vec<&str> = content.split('\n').collect();
            let label = if lines.len() == 1 {
                lines[0].rsplit('/').next().unwrap_or(lines[0]).to_string()
            } else {
                format!("{} files", lines.len())
            };
            (label.clone(), label)
        }
        ItemType::Text | ItemType::Code | ItemType::Html => {
            let first = content.split('\n').next().unwrap_or("").trim();
            let title = truncate_scalars(first, TITLE_CAP);
            let title = if title.is_empty() {
                item_type.as_str().to_string()
            } else {
                title
            };
            (title, truncate_scalars(first, PREVIEW_CAP))
        }
    }
}

/// Hostname of a URL with a leading `www.` removed (URL semantics: lowercased
/// host). `None` on parse failure, which selects the 80-scalar fallback.
fn link_host(content: &str) -> Option<String> {
    let lower = content.to_ascii_lowercase();
    let after_scheme = if lower.starts_with("https://") {
        &content["https://".len()..]
    } else if lower.starts_with("http://") {
        &content["http://".len()..]
    } else {
        return None;
    };
    let authority: &str = after_scheme
        .split(['/', '?', '#'])
        .next()
        .filter(|a| !a.is_empty())?;
    // strip userinfo and port
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let host = host_port.split(':').next().filter(|h| !h.is_empty())?;
    let host = host.to_ascii_lowercase();
    Some(host.strip_prefix("www.").unwrap_or(&host).to_string())
}
