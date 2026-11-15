//! Clipboard watcher per spec/behavior.md §2: 400 ms polling, signature
//! change detection, content selection (files > image > text), skip rules.

use crate::state::AppState;
use crate::ui::Command;
use winit::event_loop::EventLoopProxy;
use arboard::Clipboard;
use clipdeck_core::{classify, derive_pair, ItemType};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const POLL_MS: u64 = 400;
const IMAGE_RECHECK: Duration = Duration::from_secs(2);

pub fn run(state: Arc<Mutex<AppState>>, proxy: EventLoopProxy<Command>) {
    let mut last_sig = String::new();
    let mut last_image_hash = String::new();
    let mut last_image_check = Instant::now() - IMAGE_RECHECK;
    let mut first_observation = true;

    loop {
        std::thread::sleep(Duration::from_millis(POLL_MS));
        let Ok(mut clip) = Clipboard::new() else { continue };
        let text = clip.get_text().unwrap_or_default();
        let sig = sha16(text.as_bytes());

        let sig_changed = sig != last_sig;
        let mut image: Option<(usize, usize, Vec<u8>, String)> = None;
        if sig_changed || last_image_check.elapsed() >= IMAGE_RECHECK {
            last_image_check = Instant::now();
            if let Ok(img) = clip.get_image() {
                let png = encode_png(img.width, img.height, &img.bytes);
                let h = sha16(&png);
                let image_changed = h != last_image_hash && sig_changed;
                last_image_hash = h.clone();
                if image_changed {
                    image = Some((img.width, img.height, png, h));
                }
            }
        }
        drop(clip);

        if !sig_changed && image.is_none() {
            continue;
        }
        last_sig = sig;
        if first_observation {
            first_observation = false;
            continue; // baseline: whatever was on the clipboard at start is not captured (§2.1.4)
        }

        if let Some((w, h, png, _)) = image {
            capture_image(&state, &proxy, w, h, png);
        } else if !text.trim().is_empty() {
            capture_text(&state, &proxy, &text);
        }
    }
}

/// Content selection (§2.3): file URI lists become `file` items, otherwise text.
fn capture_text(state: &Arc<Mutex<AppState>>, wake: &EventLoopProxy<Command>, raw: &str) {
    let mut st = state.lock().unwrap();
    st.last_app = crate::paste::active_window_title();
    if skip_by_rules(&st) {
        st.bump();
        return;
    }
    let trimmed = raw.trim();
    if trimmed.starts_with("file://") && trimmed.lines().all(|l| l.trim().starts_with("file://")) {
        let paths: Vec<String> = trimmed
            .lines()
            .map(|l| percent_decode(l.trim().strip_prefix("file://").unwrap_or(l.trim())))
            .collect();
        let (title, preview) = derive_pair(ItemType::File, &paths.join("\n"), None);
        let content = paths.join("\n");
        let size = content.len() as u64;
        st.record(ItemType::File, content, title, preview, size, None);
    } else {
        let item_type = classify(raw);
        let (title, preview) = derive_pair(item_type, raw, None);
        let size = raw.len() as u64;
        st.record(item_type, raw.to_string(), title, preview, size, None);
    }
    drop(st);
    wake.send_event(Command::Refresh).ok();
}

fn capture_image(
    state: &Arc<Mutex<AppState>>,
    wake: &EventLoopProxy<Command>,
    w: usize,
    h: usize,
    png: Vec<u8>,
) {
    let mut st = state.lock().unwrap();
    st.last_app = crate::paste::active_window_title();
    if skip_by_rules(&st) {
        return;
    }
    let file = format!("{}.png", sha16(&png));
    let path: PathBuf = st.store.image_dir.join(&file);
    if std::fs::write(path, &png).is_err() {
        return;
    }
    let (title, preview) = derive_pair(ItemType::Image, &file, Some((w as u32, h as u32)));
    st.record(ItemType::Image, file.clone(), title, preview, png.len() as u64, Some(file));
    drop(st);
    wake.send_event(Command::Refresh).ok();
}

/// §2.2 rules 3–4 (source-app based). Linux sourceApp is the active window
/// title (X11, via the watcher's caller) or None; without attribution these
/// rules cannot match — matching is case-insensitive substring.
fn skip_by_rules(st: &AppState) -> bool {
    let Some(app) = st.last_app.as_ref().map(|a| a.to_lowercase()) else { return false };
    let s = st.settings();
    let blocked = s
        .excluded_apps
        .iter()
        .filter(|e| !e.is_empty())
        .any(|e| app.contains(&e.to_lowercase()));
    let confidential = s.ignore_confidential
        && ["keychain", "password", "1password", "bitwarden", "keepass", "lastpass"]
            .iter()
            .any(|p| app.contains(p));
    blocked || confidential
}

fn encode_png(w: usize, h: usize, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        if let Ok(mut writer) = enc.write_header() {
            writer.write_image_data(rgba).ok();
        }
    }
    out
}

fn sha16(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let full = format!("{:x}", h.finalize());
    full[..32].to_string()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
