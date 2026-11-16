//! Reusable drawing primitives (DRY): shapes, vector icons, and text with
//! verified optical centering. No application state lives here.

use fontdue::Font;
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, PremultipliedColorU8, Transform};
use unicode_bidi::BidiInfo;
use unicode_segmentation::UnicodeSegmentation;

/// Visual-center baseline bias (verified against frame dumps).
const CENTER_BIAS: f32 = 0.345;

/// Wrapped-text line advance as a multiple of `px` (single source of truth).
pub const LINE_ADVANCE: f32 = 1.55;

#[derive(Clone, Copy)]
/// Existing text roles all use the single bundled Noto Sans Arabic face.
pub enum Face {
    Regular,
    Medium,
    Mono,
}

#[derive(Clone)]
pub struct Fonts {
    pub regular: Font,
    shaper: rustybuzz::Face<'static>,
}

const NOTO_SANS_ARABIC: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/NotoSansArabic.ttf"));

enum TextGlyph {
    Font(u16),
    Key(char),
}

struct PositionedGlyph {
    glyph: TextGlyph,
    x: f32,
    y: f32,
}

struct TextLayout {
    glyphs: Vec<PositionedGlyph>,
    width: f32,
    rtl: bool,
}

impl Fonts {
    pub fn bundled() -> Fonts {
        Fonts {
            regular: Font::from_bytes(NOTO_SANS_ARABIC, fontdue::FontSettings::default()).expect("bundled font"),
            shaper: rustybuzz::Face::from_slice(NOTO_SANS_ARABIC, 0).expect("bundled shaping font"),
        }
    }

    /// Resolve visual runs without reversing source text, then shape complete
    /// runs so joining, ligatures, marks and mixed-direction numbers stay intact.
    fn layout(&self, px: f32, text: &str) -> TextLayout {
        self.layout_with_direction(px, text, None)
    }

    fn layout_with_direction(&self, px: f32, text: &str, direction: Option<unicode_bidi::Level>) -> TextLayout {
        let bidi = BidiInfo::new(text, direction);
        let mut layout = TextLayout {
            glyphs: Vec::new(), width: 0.0,
            rtl: bidi.paragraphs.first().is_some_and(|p| p.level.is_rtl()),
        };
        let scale = px / self.shaper.units_per_em() as f32;
        for paragraph in &bidi.paragraphs {
            let (levels, runs) = bidi.visual_runs(paragraph, paragraph.range.clone());
            for run in runs {
                let rtl = levels[run.start].is_rtl();
                // Keep the existing vector keyboard symbols out of font shaping.
                let mut segments = Vec::new();
                let mut start = run.start;
                for (offset, ch) in text[run.clone()].char_indices() {
                    if is_key_symbol(ch) {
                        let index = run.start + offset;
                        if start < index { segments.push((start..index, None)); }
                        segments.push((index..index + ch.len_utf8(), Some(ch)));
                        start = index + ch.len_utf8();
                    }
                }
                if start < run.end { segments.push((start..run.end, None)); }
                if rtl { segments.reverse(); }
                for (range, symbol) in segments {
                    if let Some(ch) = symbol {
                        layout.glyphs.push(PositionedGlyph { glyph: TextGlyph::Key(ch), x: layout.width, y: 0.0 });
                        layout.width += px;
                        continue;
                    }
                    let mut buffer = rustybuzz::UnicodeBuffer::new();
                    buffer.push_str(&text[range]);
                    buffer.set_direction(if rtl { rustybuzz::Direction::RightToLeft } else { rustybuzz::Direction::LeftToRight });
                    buffer.guess_segment_properties();
                    let shaped = rustybuzz::shape(&self.shaper, &[], buffer);
                    for (info, position) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
                        layout.glyphs.push(PositionedGlyph {
                            glyph: TextGlyph::Font(info.glyph_id as u16),
                            x: layout.width + position.x_offset as f32 * scale,
                            y: position.y_offset as f32 * scale,
                        });
                        layout.width += position.x_advance as f32 * scale;
                    }
                }
            }
        }
        layout
    }

    pub fn width(&self, px: f32, s: &str, _face: Face) -> f32 {
        self.layout(px, s).width
    }

    /// Draw with `baseline` as the text baseline.
    pub fn text(&self, pm: &mut Pixmap, x: f32, baseline: f32, px: f32, s: &str, _face: Face, color: [u8; 4]) {
        self.draw_layout(pm, x, baseline, px, self.layout(px, s), color);
    }

    fn draw_layout(&self, pm: &mut Pixmap, x: f32, baseline: f32, px: f32, layout: TextLayout, color: [u8; 4]) {
        for glyph in layout.glyphs {
            match glyph.glyph {
                TextGlyph::Key(ch) => key_symbol(pm, x + glyph.x, baseline - px * 0.75, px, ch, color),
                TextGlyph::Font(id) => {
                    let (m, bitmap) = self.regular.rasterize_indexed(id, px);
                    blit(pm, (x + glyph.x + m.xmin as f32) as i32,
                        (baseline - glyph.y - m.ymin as f32 - m.height as f32) as i32,
                        m.width, m.height, &bitmap, color);
                }
            }
        }
    }

    /// Left-aligned, vertically centered on `cy`.
    pub fn text_l(&self, pm: &mut Pixmap, x: f32, cy: f32, px: f32, s: &str, face: Face, color: [u8; 4]) {
        self.text(pm, x, cy + px * CENTER_BIAS, px, s, face, color);
    }

    /// Horizontally + vertically centered on (cx, cy).
    pub fn text_c(&self, pm: &mut Pixmap, cx: f32, cy: f32, px: f32, s: &str, face: Face, color: [u8; 4]) {
        let w = self.width(px, s, face);
        self.text(pm, cx - w / 2.0, cy + px * CENTER_BIAS, px, s, face, color);
    }

    /// Compute wrapped lines (single source of truth for text wrapping):
    /// word-wraps on spaces, hard-breaks words longer than `max_w`, honors `\n`,
    /// and stops at `max_lines`.
    pub fn layout_lines(&self, max_w: f32, px: f32, s: &str, face: Face, max_lines: usize) -> Vec<String> {
        let measure = |t: &str| self.width(px, t, face);
        let mut out: Vec<String> = Vec::new();
        for para in s.split('\n') {
            if out.len() >= max_lines {
                break;
            }
            // split into words, then hard-split any word wider than max_w
            let mut words: Vec<String> = Vec::new();
            for w in para.split(' ') {
                if w.is_empty() {
                    continue;
                }
                if measure(w) <= max_w {
                    words.push(w.to_string());
                    continue;
                }
                let mut chunk = String::new();
                for grapheme in w.graphemes(true) {
                    if !chunk.is_empty() && measure(&format!("{chunk}{grapheme}")) > max_w {
                        words.push(std::mem::take(&mut chunk));
                    }
                    chunk.push_str(grapheme);
                }
                if !chunk.is_empty() {
                    words.push(chunk);
                }
            }
            if words.is_empty() {
                out.push(String::new());
                continue;
            }
            let mut line = String::new();
            for w in words {
                if out.len() >= max_lines {
                    break;
                }
                let candidate = if line.is_empty() { w.clone() } else { format!("{line} {w}") };
                if measure(&candidate) > max_w && !line.is_empty() {
                    out.push(std::mem::take(&mut line));
                    line = w;
                } else {
                    line = candidate;
                }
            }
            if !line.is_empty() && out.len() < max_lines {
                out.push(line);
            }
        }
        out.truncate(max_lines);
        out
    }

    /// Word-wrapped text honoring paragraph direction and explicit newlines.
    pub fn wrap_lines(&self, pm: &mut Pixmap, x: f32, y: f32, max_w: f32, px: f32, s: &str, face: Face, max_lines: usize) {
        let mut ly = y;
        let mut remaining = max_lines;
        for paragraph in s.split('\n') {
            if remaining == 0 { break; }
            let bidi = BidiInfo::new(paragraph, None);
            let direction = bidi.paragraphs.first().map(|p| p.level);
            let lines = self.layout_lines(max_w, px, paragraph, face, remaining);
            remaining -= lines.len();
            for line in lines {
                let layout = self.layout_with_direction(px, &line, direction);
                let left = if layout.rtl { x + (max_w - layout.width).max(0.0) } else { x };
                self.draw_layout(pm, left, ly + px, px, layout, rgba(238, 240, 246, 220));
                ly += px * LINE_ADVANCE;
            }
        }
    }
}

/// Keyboard symbols use UI strokes because the bundled fonts lack these glyphs.
pub fn is_key_symbol(ch: char) -> bool {
    matches!(ch, '←' | '→' | '↵' | '↪')
}

fn key_symbol(pm: &mut Pixmap, x: f32, y: f32, px: f32, ch: char, color: [u8;4]) {
    let (tip, tail, wing, center) = if ch == '→' { (0.8,0.2,0.55,0.4) }
        else if ch == '↪' { (0.8,0.2,0.55,0.6) }
        else if ch == '↵' { (0.2,0.8,0.45,0.6) } else { (0.2,0.8,0.45,0.4) };
    line(pm, x+tip*px, y+center*px, x+tail*px, y+center*px, color);
    line(pm, x+tip*px, y+center*px, x+wing*px, y+(center-0.25)*px, color);
    line(pm, x+tip*px, y+center*px, x+wing*px, y+(center+0.25)*px, color);
    if ch == '↵' || ch == '↪' { line(pm, x+tail*px, y+0.05*px, x+tail*px, y+center*px, color); }
}

/* ---------- shapes ---------- */

pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> [u8; 4] {
    [r, g, b, a]
}

pub fn color_paint(c: [u8; 4]) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = true;
    p
}

pub fn rounded_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let k = r * 0.552_284_75;
    let (x2, y2) = (x + w, y + h);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x2 - r, y);
    pb.cubic_to(x2 - r + k, y, x2, y + r - k, x2, y + r);
    pb.line_to(x2, y2 - r);
    pb.cubic_to(x2, y2 - r + k, x2 - r + k, y2, x2 - r, y2);
    pb.line_to(x + r, y2);
    pb.cubic_to(x + r - k, y2, x, y2 - r + k, x, y2 - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

pub fn round_rect(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, c: [u8; 4]) {
    if let Some(path) = rounded_path(x, y, w, h, r) {
        pm.fill_path(&path, &color_paint(c), FillRule::Winding, Transform::identity(), None);
    }
}

pub fn stroke_round(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, c: [u8; 4], width: f32) {
    if let Some(path) = rounded_path(x, y, w, h, r) {
        pm.stroke_path(&path, &color_paint(c), &tiny_skia::Stroke { width, ..Default::default() }, Transform::identity(), None);
    }
}

pub fn line(pm: &mut Pixmap, x1: f32, y1: f32, x2: f32, y2: f32, c: [u8; 4]) {
    let mut pb = PathBuilder::new();
    pb.move_to(x1, y1);
    pb.line_to(x2, y2);
    if let Some(path) = pb.finish() {
        pm.stroke_path(&path, &color_paint(c), &tiny_skia::Stroke { width: 1.0, ..Default::default() }, Transform::identity(), None);
    }
}

fn blit(pm: &mut Pixmap, x: i32, y: i32, w: usize, h: usize, cov: &[u8], color: [u8; 4]) {
    let pw = pm.width() as i32;
    let ph = pm.height() as i32;
    let px = pm.pixels_mut();
    for row in 0..h as i32 {
        for col in 0..w as i32 {
            let (dx, dy) = (x + col, y + row);
            if dx < 0 || dy < 0 || dx >= pw || dy >= ph {
                continue;
            }
            let a = cov[row as usize * w + col as usize] as u32;
            if a == 0 {
                continue;
            }
            let idx = (dy * pw + dx) as usize;
            let p = &px[idx];
            let inv = 255 - a;
            let r = ((color[0] as u32 * a + p.red() as u32 * inv) / 255) as u8;
            let g = ((color[1] as u32 * a + p.green() as u32 * inv) / 255) as u8;
            let b = ((color[2] as u32 * a + p.blue() as u32 * inv) / 255) as u8;
            if let Some(np) = tiny_skia::PremultipliedColorU8::from_rgba(r, g, b, 255) {
                px[idx] = np;
            }
        }
    }
}

/* ---------- vector icons ---------- */

pub fn icon_more_vertical(pm: &mut Pixmap, cx: f32, cy: f32, c: [u8; 4]) {
    for offset in [-4.0, 0.0, 4.0] {
        round_rect(pm, cx - 1.1, cy + offset - 1.1, 2.2, 2.2, 1.1, c);
    }
}

pub fn icon_x(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, c: [u8; 4]) {
    line(pm, cx - r, cy - r, cx + r, cy + r, c);
    line(pm, cx + r, cy - r, cx - r, cy + r, c);
}

pub fn icon_pin(pm: &mut Pixmap, cx: f32, cy: f32, filled: bool) {
    let Some(path) = PathBuilder::from_circle(cx, cy, 4.2) else { return };
    if filled {
        pm.fill_path(&path, &color_paint(rgba(143, 176, 255, 235)), FillRule::Winding, Transform::identity(), None);
    } else {
        pm.stroke_path(&path, &color_paint(rgba(238, 240, 246, 190)), &tiny_skia::Stroke { width: 1.3, ..Default::default() }, Transform::identity(), None);
    }
}

pub fn icon_edit(pm: &mut Pixmap, cx: f32, cy: f32, c: [u8; 4]) {
    line(pm, cx - 5.5, cy + 4.5, cx + 3.0, cy - 4.0, c);
    line(pm, cx - 4.5, cy + 5.5, cx + 4.0, cy - 3.0, c);
    line(pm, cx + 2.5, cy - 5.5, cx + 5.5, cy - 5.5, c);
    line(pm, cx + 4.0, cy - 7.0, cx + 5.5, cy - 4.0, c);
}

/* ---------- helpers ---------- */

pub fn time_ago(created_at_ms: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let s = ((now - created_at_ms) / 1000).max(0);
    if s < 60 {
        "now".into()
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86400 {
        format!("{}h", s / 3600)
    } else {
        format!("{}d", s / 86400)
    }
}

pub fn parse_color(s: &str) -> [u8; 4] {
    let t = s.trim();
    if let Some(hex) = t.strip_prefix('#') {
        if hex.len() == 6 {
            if let Ok(v) = u32::from_str_radix(hex, 16) {
                return [((v >> 16) & 0xff) as u8, ((v >> 8) & 0xff) as u8, (v & 0xff) as u8, 255];
            }
        }
    }
    [120, 130, 160, 255]
}

/* ---------- layout helpers ---------- */

/// How many wrapped lines of `px`-sized text fit into `avail_h` (>= 1).
pub fn fit_lines(avail_h: f32, px: f32) -> usize {
    let line = px * LINE_ADVANCE;
    if line <= 0.0 {
        return 1;
    }
    (avail_h / line).floor().max(1.0) as usize
}

/// Contain-fit `(iw, ih)` inside `(max_w, max_h)`, preserving aspect ratio;
/// never upscales past 1:1.
pub fn fit_within(iw: f32, ih: f32, max_w: f32, max_h: f32) -> (f32, f32) {
    if iw <= 0.0 || ih <= 0.0 {
        return (0.0, 0.0);
    }
    let scale = (max_w / iw).min(max_h / ih).min(1.0);
    (iw * scale, ih * scale)
}

/// Area-average resample of `src` to `tw` × `th` pixels.
pub fn scale_box(src: &Pixmap, tw: u32, th: u32) -> Option<Pixmap> {
    if tw == 0 || th == 0 {
        return None;
    }
    let mut out = Pixmap::new(tw, th)?;
    let (sw, sh) = (src.width() as f64, src.height() as f64);
    for oy in 0..th {
        let y0 = oy as f64 * sh / th as f64;
        let y1 = (oy as f64 + 1.0) * sh / th as f64;
        for ox in 0..tw {
            let x0 = ox as f64 * sw / tw as f64;
            let x1 = (ox as f64 + 1.0) * sw / tw as f64;
            let (mut r, mut g, mut b, mut a, mut n) = (0.0, 0.0, 0.0, 0.0, 0.0);
            let iy0 = y0.floor().max(0.0) as u32;
            let iy1 = (y1.ceil().min(sh) as u32).max(iy0 + 1);
            let ix0 = x0.floor().max(0.0) as u32;
            let ix1 = (x1.ceil().min(sw) as u32).max(ix0 + 1);
            for sy in iy0..iy1.min(src.height()) {
                for sx in ix0..ix1.min(src.width()) {
                    let p = src.pixel(sx, sy)?;
                    r += p.red() as f64;
                    g += p.green() as f64;
                    b += p.blue() as f64;
                    a += p.alpha() as f64;
                    n += 1.0;
                }
            }
            if n == 0.0 {
                return None;
            }
            out.pixels_mut()[(oy * tw + ox) as usize] = PremultipliedColorU8::from_rgba(
                (r / n) as u8,
                (g / n) as u8,
                (b / n) as u8,
                (a / n) as u8,
            )?;
        }
    }
    Some(out)
}

/// Display-sized thumbnails with an LRU byte budget and bounded failed entries.
pub struct ThumbnailCache {
    images: std::collections::HashMap<(String, u32, u32), CachedThumbnail>,
    bytes: usize,
    clock: u64,
}

struct CachedThumbnail {
    fingerprint: Option<ThumbnailFingerprint>,
    pixmap: Option<Pixmap>,
    used: u64,
}

#[derive(PartialEq)]
enum ThumbnailFingerprint {
    #[cfg(test)]
    File(u64, Option<std::time::SystemTime>),
    Source(u64, u64),
}

const THUMBNAIL_BYTES: usize = 8 * 1024 * 1024;
const THUMBNAIL_ENTRIES: usize = 256;
const MAX_PREVIEW_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PREVIEW_PIXELS: u64 = 16 * 1024 * 1024;

impl Default for ThumbnailCache {
    fn default() -> Self {
        Self { images: std::collections::HashMap::new(), bytes: 0, clock: 0 }
    }
}

impl ThumbnailCache {
    fn remove(&mut self, key: &(String, u32, u32)) {
        if let Some(entry) = self.images.remove(key) {
            self.bytes -= entry.pixmap.as_ref().map_or(0, |image| image.data().len());
        }
    }

    #[cfg(test)]
    pub fn get(&mut self, directory: &std::path::Path, name: &str, width: u32, height: u32, _generation: u64) -> Option<&Pixmap> {
        let path = directory.join(name);
        let metadata = std::fs::metadata(&path).ok();
        let length = metadata.as_ref().map(|meta| meta.len());
        let fingerprint = metadata.map(|meta| ThumbnailFingerprint::File(meta.len(), meta.modified().ok()));
        self.lookup(name, width, height, length, fingerprint, || std::fs::read(path).ok())
    }

    pub fn get_image<F: FnOnce() -> Option<Vec<u8>>>(&mut self, name: &str, width: u32, height: u32,
        info: Option<(u64, u64)>, load: F) -> Option<&Pixmap> {
        let fingerprint = info.map(|(length, revision)| ThumbnailFingerprint::Source(length, revision));
        self.lookup(name, width, height, info.map(|(length, _)| length), fingerprint, load)
    }

    fn lookup<F: FnOnce() -> Option<Vec<u8>>>(&mut self, name: &str, width: u32, height: u32,
        length: Option<u64>, fingerprint: Option<ThumbnailFingerprint>, load: F) -> Option<&Pixmap> {
        if name.len() > 255 || name.is_empty() || name.contains('/') || matches!(name, "." | "..") { return None; }
        let key = (name.to_owned(), width, height);
        self.clock = self.clock.wrapping_add(1);
        if self.images.get(&key).is_some_and(|entry| entry.fingerprint != fingerprint) {
            self.remove(&key);
        }
        if !self.images.contains_key(&key) {
            let pixmap = (|| {
                if length? > MAX_PREVIEW_FILE_BYTES { return None; }
                if u64::from(width) * u64::from(height) * 4 > THUMBNAIL_BYTES as u64 { return None; }
                let bytes = load()?;
                if bytes.len() as u64 > MAX_PREVIEW_FILE_BYTES { return None; }
                let reader = png::Decoder::new(std::io::Cursor::new(&bytes)).read_info().ok()?;
                if u64::from(reader.info().width) * u64::from(reader.info().height) > MAX_PREVIEW_PIXELS { return None; }
                drop(reader);
                let decoded = Pixmap::decode_png(&bytes).ok()?;
                let (w, h) = fit_within(decoded.width() as f32, decoded.height() as f32, width as f32, height as f32);
                scale_box(&decoded, w.round() as u32, h.round() as u32)
            })();
            let bytes = pixmap.as_ref().map_or(0, |image| image.data().len());
            while self.images.len() >= THUMBNAIL_ENTRIES || self.bytes + bytes > THUMBNAIL_BYTES {
                let Some(oldest) = self.images.iter().min_by_key(|(_, entry)| entry.used).map(|(key, _)| key.clone()) else { break };
                self.remove(&oldest);
            }
            self.bytes += bytes;
            self.images.insert(key.clone(), CachedThumbnail { fingerprint, pixmap, used: self.clock });
        }
        let entry = self.images.get_mut(&key).expect("inserted thumbnail");
        entry.used = self.clock;
        entry.pixmap.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_cache_bounds_failed_load_entries() {
        let mut cache = ThumbnailCache::default();
        for n in 0..300 {
            assert!(cache.get(std::path::Path::new("/nonexistent-clipdeck-cache-test"), &format!("{n}.png"), 64, 64, 1).is_none());
        }
        assert!(cache.images.len() <= 256, "failed image loads grow without a bound");
    }

    #[test]
    fn image_sources_fetch_only_on_a_cache_miss_or_revision_change() {
        let mut cache = ThumbnailCache::default();
        let calls = std::cell::Cell::new(0);
        let bytes = include_bytes!("../assets/icon.png");
        for _ in 0..3 {
            assert!(cache.get_image("blob.png", 64, 64, Some((bytes.len() as u64, 1)), || {
                calls.set(calls.get() + 1);
                Some(bytes.to_vec())
            }).is_some());
        }
        assert_eq!(calls.get(), 1);
        cache.get_image("blob.png", 64, 64, Some((bytes.len() as u64, 2)), || {
            calls.set(calls.get() + 1);
            Some(bytes.to_vec())
        });
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn thumbnail_cache_bounds_decoded_bytes() {
        let dir = std::env::temp_dir().join(format!("clipdeck-cache-{}", clipdeck_core::store::uuid_v4ish()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut cache = ThumbnailCache::default();
        for n in 0..80 {
            let name = format!("{n}.png");
            std::fs::write(dir.join(&name), include_bytes!("../assets/icon.png")).unwrap();
            assert!(cache.get(&dir, &name, 200, 200, 1).is_some());
        }
        let bytes: usize = cache.images.values().filter_map(|entry| entry.pixmap.as_ref()).map(|pixmap| pixmap.data().len()).sum();
        assert!(bytes <= 8 * 1024 * 1024, "decoded images exceed the cache byte budget: {bytes}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn persian_letters_render_distinct_glyphs_in_every_face() {
        let fonts = Fonts::bundled();
        for face in [Face::Regular, Face::Medium, Face::Mono] {
            let render = |text: &str| {
                let mut pm = Pixmap::new(80, 40).unwrap();
                fonts.text(&mut pm, 4.0, 28.0, 20.0, text, face, rgba(255,255,255,255));
                pm.data().to_vec()
            };
            assert_ne!(render("س"), render("م"), "Persian letters must not share the missing-glyph box");
            assert_ne!(render("ی"), render("\u{10ffff}"), "Persian yeh needs a real glyph");
        }
    }

    #[test]
    fn persian_words_are_shaped_instead_of_drawn_as_isolated_characters() {
        let fonts = Fonts::bundled();
        let mut joined = Pixmap::new(160, 50).unwrap();
        let mut isolated = joined.clone();
        fonts.text(&mut joined, 4.0, 32.0, 24.0, "سلام", Face::Regular, rgba(255,255,255,255));
        let mut x = 4.0;
        for ch in "سلام".chars() {
            let text = ch.to_string();
            fonts.text(&mut isolated, x, 32.0, 24.0, &text, Face::Regular, rgba(255,255,255,255));
            x += fonts.width(24.0, &text, Face::Regular);
        }
        assert_ne!(joined.data(), isolated.data(), "Arabic-script text needs contextual shaping and RTL ordering");
    }

    #[test]
    fn wrapped_persian_text_is_aligned_to_the_right_edge() {
        let fonts = Fonts::bundled();
        let mut pm = Pixmap::new(220, 80).unwrap();
        fonts.wrap_lines(&mut pm, 10.0, 10.0, 200.0, 18.0, "سلام دنیا", Face::Regular, 2);
        let ink: Vec<_> = pm.pixels().iter().enumerate().filter(|(_, p)| p.alpha() > 0)
            .map(|(i, _)| (i % 220, i / 220)).collect();
        assert!(!ink.is_empty());
        assert!(ink.iter().all(|(x, y)| *x >= 10 && *x < 210 && *y >= 10 && *y < 80));
        assert!(ink.iter().map(|(x, _)| *x).max().unwrap() >= 204, "RTL text must end at the right margin");
    }

    #[test]
    fn wrapped_latin_line_keeps_its_persian_paragraph_direction() {
        let fonts = Fonts::bundled();
        let mut pm = Pixmap::new(220, 140).unwrap();
        fonts.wrap_lines(&mut pm, 10.0, 10.0, 100.0, 18.0, "سلام دنیا ClipDeck", Face::Regular, 4);
        let last_line = fonts.layout_lines(100.0, 18.0, "سلام دنیا ClipDeck", Face::Regular, 4).len() - 1;
        let y = (10.0 + last_line as f32 * 18.0 * LINE_ADVANCE) as usize;
        let rightmost = pm.pixels().iter().enumerate().filter(|(i, p)| i / 220 >= y && p.alpha() > 0)
            .map(|(i, _)| i % 220).max().unwrap();
        assert!(rightmost >= 104, "wrapped Latin text must retain the RTL paragraph margin, got {rightmost}");
    }

    #[test]
    fn mixed_persian_text_keeps_latin_and_numbers_in_reading_order() {
        let fonts = Fonts::bundled();
        let glyphs = |text: &str| fonts.layout(18.0, text).glyphs.into_iter().filter_map(|g| match g.glyph {
            TextGlyph::Font(id) => Some(id), TextGlyph::Key(_) => None,
        }).collect::<Vec<_>>();
        let latin = glyphs("ClipDeck 123");
        let mixed = glyphs("سلام ClipDeck 123");
        assert!(mixed.starts_with(&latin), "the LTR run belongs on the left, without reversing its letters or numbers");
        assert!(mixed.iter().all(|id| *id != 0), "no missing glyphs in mixed Persian/Latin text");
        let isolated = [glyphs("ل"), glyphs("ا")].concat();
        assert_ne!(glyphs("لا"), isolated, "lam-alef must use contextual glyphs, regardless of how the font represents the ligature");
    }

    #[test]
    fn persian_wrapping_preserves_logical_text_and_combining_marks() {
        let fonts = Fonts::bundled();
        let text = "سَلام می\u{200c}روم ClipDeck 123";
        let lines = fonts.layout_lines(100.0, 16.0, text, Face::Regular, 20);
        assert_eq!(lines.join(" "), text);
        for line in lines {
            assert!(fonts.width(16.0, &line, Face::Regular) <= 100.0);
            assert!(fonts.layout(16.0, &line).glyphs.iter().all(|g| !matches!(g.glyph, TextGlyph::Font(0))));
        }
        let joined = fonts.layout(18.0, "میروم");
        let unjoined = fonts.layout(18.0, "می\u{200c}روم");
        assert_ne!(joined.width, unjoined.width, "ZWNJ must affect contextual joining, without changing the stored string");
    }

    #[test]
    fn keyboard_symbols_have_distinct_shapes_and_fixed_width() {
        let fonts = Fonts::bundled();
        let mut rendered = Vec::new();
        for ch in ['←', '→', '↵', '↪'] {
            assert_eq!(fonts.width(12.0, &ch.to_string(), Face::Medium), 12.0);
            let mut pm = Pixmap::new(24, 24).unwrap();
            fonts.text(&mut pm, 4.0, 18.0, 12.0, &ch.to_string(), Face::Medium, rgba(255,255,255,255));
            let ink: Vec<_> = pm.pixels().iter().enumerate().filter(|(_,p)| p.alpha()>0).map(|(i,_)| (i%24,i/24)).collect();
            assert!(ink.len()>8, "symbol has no visible strokes: {ch}");
            assert!(ink.iter().all(|(x,y)| (4..16).contains(x) && (8..20).contains(y)));
            rendered.push(pm.data().to_vec());
        }
        for i in 0..rendered.len() {
            for j in i+1..rendered.len() { assert_ne!(rendered[i],rendered[j]); }
        }
    }

    #[test]
    fn fit_lines_uses_line_advance() {
        // advance = px * 1.55
        assert_eq!(fit_lines(13.0 * 1.55 * 4.0, 13.0), 4);
        assert_eq!(fit_lines(13.0 * 1.55 * 4.0 - 1.0, 13.0), 3);
        assert_eq!(fit_lines(1.0, 13.0), 1);
    }

    #[test]
    fn fit_within_preserves_aspect_and_contains() {
        let (w, h) = fit_within(200.0, 100.0, 100.0, 100.0);
        assert!((w - 100.0).abs() < 1e-4 && (h - 50.0).abs() < 1e-4);
        let (w, h) = fit_within(100.0, 200.0, 100.0, 100.0);
        assert!((w - 50.0).abs() < 1e-4 && (h - 100.0).abs() < 1e-4);
        // never upscales past 1:1
        let (w, h) = fit_within(10.0, 10.0, 100.0, 100.0);
        assert!((w - 10.0).abs() < 1e-4 && (h - 10.0).abs() < 1e-4);
    }

    #[test]
    fn scale_box_downsamples_by_area_average() {
        // 2×2: red, green / blue, white  →  1×1 = average
        let mut src = Pixmap::new(2, 2).unwrap();
        let px = src.pixels_mut();
        px[0] = PremultipliedColorU8::from_rgba(255, 0, 0, 255).unwrap();
        px[1] = PremultipliedColorU8::from_rgba(0, 255, 0, 255).unwrap();
        px[2] = PremultipliedColorU8::from_rgba(0, 0, 255, 255).unwrap();
        px[3] = PremultipliedColorU8::from_rgba(255, 255, 255, 255).unwrap();
        let out = scale_box(&src, 1, 1).unwrap();
        let p = out.pixel(0, 0).unwrap();
        assert_eq!(
            (p.red(), p.green(), p.blue(), p.alpha()),
            (127, 127, 127, 255)
        );
    }

    #[test]
    fn scale_box_identity_roundtrip() {
        let mut src = Pixmap::new(2, 1).unwrap();
        let px = src.pixels_mut();
        px[0] = PremultipliedColorU8::from_rgba(10, 20, 30, 255).unwrap();
        px[1] = PremultipliedColorU8::from_rgba(40, 50, 60, 255).unwrap();
        let out = scale_box(&src, 2, 1).unwrap();
        let (a, b) = (out.pixel(0, 0).unwrap(), out.pixel(1, 0).unwrap());
        assert_eq!((a.red(), a.green(), a.blue()), (10, 20, 30));
        assert_eq!((b.red(), b.green(), b.blue()), (40, 50, 60));
    }

    #[test]
    fn layout_lines_breaks_long_words_and_bounds_width() {
        let fonts = Fonts::bundled();
        let s = "short words then XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX and more";
        let lines = fonts.layout_lines(120.0, 12.0, s, Face::Regular, 20);
        assert!(lines.len() > 1);
        assert!(lines.len() <= 20);
        for l in &lines {
            assert!(fonts.width(12.0, l, Face::Regular) <= 120.0 + 1.0, "line too wide: {l:?}");
        }
    }

    #[test]
    fn layout_lines_respects_max_lines() {
        let fonts = Fonts::bundled();
        let s = "one two three four five six seven eight nine ten";
        let lines = fonts.layout_lines(60.0, 12.0, s, Face::Regular, 3);
        assert!(lines.len() <= 3);
    }
}
