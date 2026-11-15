//! Clipboard write + paste simulation per spec/behavior.md §7.

use crate::state::AppState;
use arboard::Clipboard;
use clipdeck_core::{ClipItem, Store};
use std::process::Command;

/// §7.2: image → PNG payload; html → rich text when !plain and markup
/// present; everything else (or any plain case) → plain text.
pub fn write_item(store: &Store, item: &ClipItem, plain: bool) {
    let Ok(mut clip) = Clipboard::new() else { return };
    if !plain && item.item_type == clipdeck_core::ItemType::Image {
        if let Some(name) = item.image_file.as_ref() {
            if let Ok(png) = std::fs::read(store.image_dir.join(name)) {
                if let Some((w, h, rgba)) = decode_png(&png) {
                    clip.set_image(arboard::ImageData { width: w, height: h, bytes: std::borrow::Cow::Owned(rgba) })
                        .ok();
                    return;
                }
            }
        }
    }
    clip.set_text(item.content.clone()).ok();
}

/// §7.4: simulate the system paste keystroke into the previously focused app.
pub fn simulate_paste() {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        if try_exec("wtype", &["-M", "ctrl", "-k", "v", "-m", "ctrl"]) {
            return;
        }
        if try_exec("ydotool", &["key", "29:1", "47:1", "47:0", "29:0"]) {
            return;
        }
    }
    try_exec("xdotool", &["key", "--clearmodifiers", "ctrl+v"]);
}

/// Active window title (X11) for sourceApp attribution; None when unknown.
pub fn active_window_title() -> Option<String> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return None;
    }
    let out = Command::new("xdotool")
        .args(["getactivewindow", "getwindowname"])
        .output()
        .ok()?;
    let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

fn try_exec(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd).args(args).output().map(|o| o.status.success()).unwrap_or(false)
}

fn decode_png(png: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let mut rgba = buf[..info.buffer_size()].to_vec();
    if info.color_type == png::ColorType::Rgb {
        // expand to rgba
        let mut out = Vec::with_capacity(rgba.len() / 3 * 4);
        for c in rgba.chunks_exact(3) {
            out.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
        rgba = out;
    }
    Some((info.width as usize, info.height as usize, rgba))
}

/// Full §7 paste flow used by the UI: write clipboard + hooks + hide + keystroke.
pub fn perform_paste(state: &mut AppState, id: &str, force_plain: Option<bool>) {
    if state.paste_item(id, force_plain) {
        simulate_paste();
    }
}
