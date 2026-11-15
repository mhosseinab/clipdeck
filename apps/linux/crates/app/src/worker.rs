//! Clipboard worker — the SOLE owner of arboard and window-title queries.
//! Reads produce `AppEvent::Captured`; writes arrive as `WorkerCmd::Write` and
//! complete as `AppEvent::Written`. No locks live here; communication is by
//! channel + event proxy only.

use crate::events::{AppEvent, WritePayload, WritePurpose};
use clipdeck_core::ClipItem;
use arboard::Clipboard;
use clipdeck_core::{classify, derive_pair, ItemType};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};
use winit::event_loop::EventLoopProxy;

const POLL_MS: u64 = 400;
const IMAGE_RECHECK: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub enum WorkerCmd {
    SetWatching(bool),
    Write { payload: WritePayload, purpose: WritePurpose },
}

pub struct Worker {
    events: EventLoopProxy<AppEvent>,
    cmd_rx: Receiver<WorkerCmd>,
    image_dir: PathBuf,
    clip: Option<Clipboard>,
    last_sig: String,
    last_image_hash: String,
    last_image_check: Instant,
    capture_policy: CapturePolicy,
}

pub fn spawn(
    events: EventLoopProxy<AppEvent>,
    cmd_rx: Receiver<WorkerCmd>,
    image_dir: PathBuf,
    watching: bool,
) {
    let mut worker = Worker {
        events,
        cmd_rx,
        image_dir,
        clip: None,
        last_sig: String::new(),
        last_image_hash: String::new(),
        last_image_check: Instant::now() - IMAGE_RECHECK,
        capture_policy: CapturePolicy::new(watching),
    };
    std::thread::spawn(move || worker.run());
}

impl Worker {
    fn clipboard(&mut self) -> Option<&mut Clipboard> {
        if self.clip.is_none() {
            self.clip = Clipboard::new().ok();
        }
        self.clip.as_mut()
    }

    fn run(&mut self) {
        loop {
            match self.cmd_rx.recv_timeout(Duration::from_millis(POLL_MS)) {
                Ok(WorkerCmd::Write { payload, purpose }) => self.write(payload, purpose),
                Ok(WorkerCmd::SetWatching(watching)) => {
                    self.capture_policy.set_watching(watching);
                    if watching {
                        self.last_image_check = Instant::now() - IMAGE_RECHECK;
                    }
                }
                Err(RecvTimeoutError::Timeout) => self.poll(),
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    fn write(&mut self, payload: WritePayload, purpose: WritePurpose) {
        if let Some(clip) = self.clipboard() {
            match payload {
                WritePayload::Text(t) => {
                    let _ = clip.set_text(t);
                }
                WritePayload::Image { width, height, rgba } => {
                    let _ = clip.set_image(arboard::ImageData {
                        width,
                        height,
                        bytes: std::borrow::Cow::Owned(rgba),
                    });
                }
            }
        }
        self.events.send_event(AppEvent::Written { purpose }).ok();
    }

    /// §2 capture: 400 ms polling, signature change, files > image > text.
    fn poll(&mut self) {
        // short, disjoint borrows of the clipboard; state updates happen after
        let text = {
            let Some(clip) = self.clipboard() else { return };
            clip.get_text().unwrap_or_default()
        };
        let sig = sha16(text.as_bytes());
        let sig_changed = sig != self.last_sig;
        let mut image: Option<(usize, usize, Vec<u8>)> = None;
        if sig_changed || self.last_image_check.elapsed() >= IMAGE_RECHECK {
            self.last_image_check = Instant::now();
            if let Some(clip) = self.clipboard() {
                if let Ok(img) = clip.get_image() {
                    let png = encode_png(img.width, img.height, &img.bytes);
                    let h = sha16(&png);
                    let image_changed = image_changed(&self.last_image_hash, &h);
                    self.last_image_hash = h;
                    if image_changed {
                        image = Some((img.width, img.height, png));
                    }
                }
            }
        }
        if !sig_changed && image.is_none() && !self.capture_policy.first_observation {
            return;
        }
        self.last_sig = sig;
        if !self.capture_policy.should_capture_change() {
            return; // Startup/resume baseline or paused: never persist image content.
        }
        if let Some((w, h, png)) = image {
            self.capture_image(w, h, png);
        } else if !text.trim().is_empty() {
            self.capture_text(&text);
        }
    }

    fn capture_text(&mut self, raw: &str) {
        let trimmed = raw.trim();
        let item = if trimmed.starts_with("file://")
            && trimmed.lines().all(|l| l.trim().starts_with("file://"))
        {
            let paths: Vec<String> = trimmed
                .lines()
                .map(|l| percent_decode(l.trim().strip_prefix("file://").unwrap_or(l.trim())))
                .collect();
            let content = paths.join("\n");
            let (title, preview) = derive_pair(ItemType::File, &content, None);
            let size = content.len() as u64;
            ClipItem {
                id: clipdeck_core::store::uuid_v4ish(),
                item_type: ItemType::File,
                content,
                preview,
                title,
                source_app: None,
                created_at: now_ms(),
                size_bytes: size,
                pinned: false,
                pinboard_id: None,
                stack_order: None,
                image_file: None,
            }
        } else {
            let item_type = classify(raw);
            let (title, preview) = derive_pair(item_type, raw, None);
            ClipItem {
                id: clipdeck_core::store::uuid_v4ish(),
                item_type,
                content: raw.to_string(),
                preview,
                title,
                source_app: None,
                created_at: now_ms(),
                size_bytes: raw.len() as u64,
                pinned: false,
                pinboard_id: None,
                stack_order: None,
                image_file: None,
            }
        };
        self.send_captured(item);
    }

    fn capture_image(&mut self, w: usize, h: usize, png: Vec<u8>) {
        let file = format!("{}.png", sha16(&png));
        if std::fs::write(self.image_dir.join(&file), &png).is_err() {
            return;
        }
        let (title, preview) = derive_pair(ItemType::Image, &file, Some((w as u32, h as u32)));
        let item = ClipItem {
            id: clipdeck_core::store::uuid_v4ish(),
            item_type: ItemType::Image,
            content: file.clone(),
            preview,
            title,
            source_app: None,
            created_at: now_ms(),
            size_bytes: png.len() as u64,
            pinned: false,
            pinboard_id: None,
            stack_order: None,
            image_file: Some(file),
        };
        self.send_captured(item);
    }

    fn send_captured(&mut self, item: ClipItem) {
        let source_app = active_window_title();
        self.events.send_event(AppEvent::Captured { item, source_app }).ok();
    }
}

/// Active window title (X11) for sourceApp attribution; None when unknown.
fn active_window_title() -> Option<String> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return None;
    }
    let out = std::process::Command::new("xdotool")
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

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn encode_png(w: usize, h: usize, rgba: &[u8]) -> Vec<u8> {
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

pub fn decode_png(png: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let mut rgba = buf[..info.buffer_size()].to_vec();
    if info.color_type == png::ColorType::Rgb {
        let mut out = Vec::with_capacity(rgba.len() / 3 * 4);
        for c in rgba.chunks_exact(3) {
            out.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
        rgba = out;
    }
    Some((info.width as usize, info.height as usize, rgba))
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

struct CapturePolicy {
    watching: bool,
    first_observation: bool,
}

impl CapturePolicy {
    fn new(watching: bool) -> Self {
        Self { watching, first_observation: true }
    }

    fn set_watching(&mut self, watching: bool) {
        if watching && !self.watching {
            self.first_observation = true;
        }
        self.watching = watching;
    }

    fn should_capture_change(&mut self) -> bool {
        let baseline = self.first_observation;
        self.first_observation = false;
        self.watching && !baseline
    }
}

fn image_changed(previous_hash: &str, current_hash: &str) -> bool {
    current_hash != previous_hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paused_images_are_not_persisted_and_resume_starts_with_a_baseline() {
        let mut policy = CapturePolicy::new(true);
        assert!(!policy.should_capture_change(), "startup is baseline only");
        assert!(policy.should_capture_change());
        policy.set_watching(false);
        assert!(!policy.should_capture_change(), "paused observations must not reach persistence");
        policy.set_watching(true);
        assert!(!policy.should_capture_change(), "resume must not record content copied while paused");
        assert!(policy.should_capture_change(), "new copies after the resume baseline are captured");
    }

    #[test]
    fn consecutive_image_copies_do_not_require_text_changes() {
        // Pixel buffers are test data; no desktop clipboard or generated artwork.
        let first = sha16(&encode_png(1, 1, &[10, 20, 30, 255]));
        let second = sha16(&encode_png(1, 1, &[30, 20, 10, 255]));
        assert!(image_changed("", &first));
        assert!(image_changed(&first, &second), "a second image-only copy must be captured");
        assert!(!image_changed(&second, &second), "unchanged images must not be recaptured");
    }
}
