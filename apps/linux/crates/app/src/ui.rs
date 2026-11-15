//! Overlay UI: custom-drawn window (winit + softbuffer + tiny-skia + fontdue).
//! Layout and keyboard behavior follow spec/behavior.md §13.

use crate::paste;
use crate::state::AppState;
use clipdeck_core::{pretty_accel, record_accel, ClipItem, ItemType, KeyPress};
use fontdue::Font;
use std::sync::{Arc, Mutex};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

const W: f32 = 1240.0;
const H: f32 = 372.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Toggle,
    OpenSettings,
    Quit,
    Refresh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hit {
    Select(usize),
    Paste(usize),
    Copy(usize),
    Delete(usize),
    Pin(usize),
    Edit(usize),
    OpenSettings,
    StackToggle,
    StackPasteNext,
    StackClear,
    StackRemove(usize),
    SaveEdit,
    CancelEdit,
    NewPinboard,
    SetFilter(usize), // 0=history 1=pinned 2..=pinboards
    Toggle(&'static str),
    ShortcutCapture,
    Export,
    Import,
}

pub struct Ui {
    state: Arc<Mutex<AppState>>,
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    font: Font,
    mono: Font,
    query: String,
    selected: usize,
    edit: Option<(String, String)>,
    settings: bool,
    capture: bool,
    filter: usize,
    stack_open: bool,
    hits: Vec<(f32, f32, f32, f32, Hit)>,
    items: Vec<ClipItem>,
    mouse: Option<(f32, f32)>,
    visible: bool,
    mods: ModifiersState,
}

fn load_font(mono: bool) -> Font {
    let candidates = if mono {
        vec![
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
        ]
    } else {
        vec![
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        ]
    };
    for p in candidates {
        if let Ok(bytes) = std::fs::read(p) {
            if let Ok(f) = Font::from_bytes(bytes, fontdue::FontSettings::default()) {
                return f;
            }
        }
    }
    // last resort: any ttf under /usr/share/fonts
    if let Some(bytes) = find_any_ttf() {
        if let Ok(f) = Font::from_bytes(bytes, fontdue::FontSettings::default()) {
            return f;
        }
    }
    panic!("no usable TTF font found under /usr/share/fonts");
}

fn find_any_ttf() -> Option<Vec<u8>> {
    let mut stack = vec![std::path::PathBuf::from("/usr/share/fonts")];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().map(|x| x == "ttf").unwrap_or(false) {
                    return std::fs::read(p).ok();
                }
            }
        }
    }
    None
}

pub fn run(
    state: Arc<Mutex<AppState>>,
    event_loop: EventLoop<Command>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut ui = Ui {
        state,
        window: None,
        surface: None,
        font: load_font(false),
        mono: load_font(true),
        query: String::new(),
        selected: 0,
        edit: None,
        settings: false,
        capture: false,
        filter: 0,
        stack_open: false,
        hits: Vec::new(),
        items: Vec::new(),
        mouse: None,
        visible: true,
        mods: ModifiersState::empty(),
    };
    event_loop.run_app(&mut ui)?;
    Ok(())
}

impl ApplicationHandler<Command> for Ui {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("ClipDeck")
            .with_inner_size(winit::dpi::LogicalSize::new(W, H))
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false);
        let window = Arc::new(el.create_window(attrs).expect("window"));
        let context = softbuffer::Context::new(window.clone()).expect("softbuffer context");
        let surface = softbuffer::Surface::new(&context, window.clone()).expect("surface");
        self.window = Some(window);
        self.surface = Some(surface);
        self.refresh_items();
    }

    fn user_event(&mut self, _el: &ActiveEventLoop, event: Command) {
        match event {
            Command::Quit => {
                self.state.lock().unwrap().store.save();
                std::process::exit(0);
            }
            Command::Toggle => {
                if let Some(w) = &self.window {
                    if self.visible {
                        w.set_visible(false);
                        self.visible = false;
                    } else {
                        w.set_visible(true);
                        w.focus_window();
                        self.visible = true;
                        self.refresh_items();
                    }
                }
            }
            Command::OpenSettings => {
                self.settings = true;
                self.stack_open = false;
                if let Some(w) = &self.window {
                    w.set_visible(true);
                    w.focus_window();
                    self.visible = true;
                }
            }
            Command::Refresh => self.refresh_items(),
        }
        self.request_redraw();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.state.lock().unwrap().store.save();
                el.exit();
            }
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::Resized(size) => {
                if let (Some(surface), Some(w)) = (self.surface.as_mut(), self.window.as_ref()) {
                    if let (Some(width), Some(height)) = (
                        std::num::NonZeroU32::new(size.width),
                        std::num::NonZeroU32::new(size.height),
                    ) {
                        surface.resize(width, height).ok();
                    }
                    w.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse = Some((position.x as f32, position.y as f32));
            }
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                self.click();
            }
            WindowEvent::KeyboardInput { event, .. } => self.key(event),
            _ => {}
        }
    }
}

impl Ui {
    fn request_redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn refresh_items(&mut self) {
        let state_arc = self.state.clone();
        let st = state_arc.lock().unwrap();
        let q = self.query.to_lowercase();
        let mut items: Vec<ClipItem> = st
            .store
            .items
            .iter()
            .filter(|i| match self.filter {
                1 => i.pinned,
                n if n >= 2 => {
                    st.store
                        .pinboards
                        .get(n - 2)
                        .map(|p| i.pinboard_id.as_deref() == Some(p.id.as_str()))
                        .unwrap_or(false)
                }
                _ => true,
            })
            .filter(|i| {
                q.is_empty()
                    || i.title.to_lowercase().contains(&q)
                    || i.preview.to_lowercase().contains(&q)
                    || i.content.to_lowercase().contains(&q)
            })
            .cloned()
            .collect();
        items.truncate(300);
        self.items = items;
        if self.selected >= self.items.len() {
            self.selected = 0;
        }
    }

    /* ---------- interaction ---------- */

    fn click(&mut self) {
        // hit rects are in buffer pixels == CSS pixels here (logical size)
        let Some((x, y)) = self.last_mouse() else { return };
        let hits = std::mem::take(&mut self.hits);
        let hit = hits
            .iter()
            .find(|(hx, hy, hw, hh, _)| x >= *hx && x <= hx + hw && y >= *hy && y <= hy + hh)
            .map(|(_, _, _, _, h)| *h);
        self.hits = hits;
        let Some(hit) = hit else { return };
        let state_arc = self.state.clone();
        let mut st = state_arc.lock().unwrap();
        match hit {
            Hit::Select(i) => self.selected = i,
            Hit::Paste(i) => {
                if let Some(it) = self.items.get(i) {
                    let id = it.id.clone();
                    drop(st);
                    self.do_paste(&id, None);
                    return;
                }
            }
            Hit::Copy(i) => {
                if let Some(it) = self.items.get(i) {
                    st.copy_item(&it.id);
                    crate::sounds::play("copy");
                }
            }
            Hit::Delete(i) => {
                if let Some(it) = self.items.get(i) {
                    st.store.remove(&it.id);
                    st.store.save();
                    st.bump();
                }
            }
            Hit::Pin(i) => {
                if let Some(it) = self.items.get(i) {
                    let pinned = !it.pinned;
                    st.store.update(&it.id, |i| i.pinned = pinned);
                    st.store.save();
                    st.bump();
                }
            }
            Hit::Edit(i) => {
                if let Some(it) = self.items.get(i) {
                    self.edit = Some((it.id.clone(), it.content.clone()));
                }
            }
            Hit::OpenSettings => {
                self.settings = true;
                self.stack_open = false;
            }
            Hit::StackToggle => self.stack_open = !self.stack_open,
            Hit::StackPasteNext => {
                drop(st);
                self.do_stack_pop();
                return;
            }
            Hit::StackClear => {
                st.store.clear_stack();
                st.store.save();
                st.bump();
            }
            Hit::StackRemove(i) => {
                let queued = st.store.stack_items();
                if let Some(it) = queued.get(i) {
                    let id = it.id.clone();
                    st.store.unstack(&id);
                    st.store.save();
                    st.bump();
                }
            }
            Hit::SaveEdit => {
                if let Some((id, draft)) = self.edit.take() {
                    st.store.update(&id, |i| {
                        let (title, preview) =
                            clipdeck_core::derive_pair(i.item_type, &draft, None);
                        i.content = draft;
                        i.title = title;
                        i.preview = preview;
                    });
                    st.store.save();
                    st.bump();
                }
            }
            Hit::CancelEdit => self.edit = None,
            Hit::NewPinboard => {
                drop(st);
                return;
            }
            Hit::SetFilter(n) => {
                self.filter = n;
                self.selected = 0;
            }
            Hit::Toggle(key) => {
                let s = &mut st.store.settings;
                match key {
                    "watch" => s.watch_enabled = !s.watch_enabled,
                    "sound_copy" => s.sound_copy = !s.sound_copy,
                    "sound_paste" => s.sound_paste = !s.sound_paste,
                    "transient" => s.ignore_transient = !s.ignore_transient,
                    "confidential" => s.ignore_confidential = !s.ignore_confidential,
                    "link_previews" => s.generate_link_previews = !s.generate_link_previews,
                    "plain" => s.paste_as_plain_text = !s.paste_as_plain_text,
                    "login" => {
                        s.launch_at_login = !s.launch_at_login;
                        let on = s.launch_at_login;
                        let exe = std::env::current_exe()
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let home = std::env::var_os("HOME")
                            .map(std::path::PathBuf::from)
                            .unwrap_or_default();
                        clipdeck_core::apply_login_item_linux(on, &exe, &home).ok();
                    }
                    "share" => s.show_in_screen_share = !s.show_in_screen_share,
                    "stack_dir" => {
                        s.stack_direction =
                            if s.stack_direction == "fifo" { "lifo".into() } else { "fifo".into() }
                    }
                    _ => {}
                }
                st.store.save();
                st.bump();
            }
            Hit::ShortcutCapture => self.capture = true,
            Hit::Export => {
                let json = st.store.export_json();
                std::fs::write("clipdeck-export.json", json).ok();
            }
            Hit::Import => {
                if let Ok(raw) = std::fs::read_to_string("clipdeck-export.json") {
                    st.store.import_json(&raw);
                    st.store.save();
                    st.bump();
                }
            }
        }
        drop(st);
        self.refresh_items();
        self.request_redraw();
    }

    fn do_paste(&mut self, id: &str, plain: Option<bool>) {
        {
            let state_arc = self.state.clone();
        let mut st = state_arc.lock().unwrap();
            paste::perform_paste(&mut *st, id, plain);
        }
        if let Some(w) = &self.window {
            w.set_visible(false);
            self.visible = false;
        }
    }

    fn do_stack_pop(&mut self) {
        {
            let state_arc = self.state.clone();
        let mut st = state_arc.lock().unwrap();
            if st.pop_stack() {
                paste::simulate_paste();
            }
        }
        if let Some(w) = &self.window {
            w.set_visible(false);
        }
    }

    fn last_mouse(&self) -> Option<(f32, f32)> {
        self.mouse
    }

    fn key(&mut self, ev: KeyEvent) {
        if ev.state != ElementState::Pressed {
            return;
        }
        let ctrl = self.mods.contains(ModifiersState::CONTROL);
        let shift = self.mods.contains(ModifiersState::SHIFT);

        if self.capture {
            if let Key::Named(NamedKey::Escape) = ev.logical_key {
                self.capture = false;
                self.request_redraw();
                return;
            }
            if let Some(accel) = accel_from_key_event(&ev, self.mods) {
                let state_arc = self.state.clone();
                let mut st = state_arc.lock().unwrap();
                let old = st.store.settings.shortcut.clone();
                // best-effort live re-registration
                if let Ok(mgr) = global_hotkey::GlobalHotKeyManager::new() {
                    if crate::hotkey::reregister(&mgr, &old, &accel) {
                        st.store.settings.shortcut = accel;
                        st.store.save();
                    }
                } else {
                    st.store.settings.shortcut = accel;
                    st.store.save();
                }
                self.capture = false;
            }
            self.request_redraw();
            return;
        }

        if let Some((_id, draft)) = &mut self.edit {
            match &ev.logical_key {
                Key::Named(NamedKey::Escape) => self.edit = None,
                Key::Named(NamedKey::Enter) => {
                    let (id, content) = self.edit.take().unwrap();
                    let state_arc = self.state.clone();
        let mut st = state_arc.lock().unwrap();
                    st.store.update(&id, |i| {
                        let (title, preview) = clipdeck_core::derive_pair(i.item_type, &content, None);
                        i.content = content.clone();
                        i.title = title;
                        i.preview = preview;
                    });
                    st.store.save();
                    st.bump();
                    self.refresh_items();
                }
                Key::Named(NamedKey::Backspace) => {
                    draft.pop();
                }
                Key::Character(c) => draft.push_str(c),
                _ => {}
            }
            self.request_redraw();
            return;
        }

        match &ev.logical_key {
            Key::Named(NamedKey::Escape) => {
                if self.settings {
                    self.settings = false;
                } else if self.stack_open {
                    self.stack_open = false;
                } else if let Some(w) = &self.window {
                    w.set_visible(false);
                    self.visible = false;
                }
            }
            Key::Named(NamedKey::ArrowRight) | Key::Named(NamedKey::Tab) if !shift => {
                self.selected = (self.selected + 1).min(self.items.len().saturating_sub(1));
            }
            Key::Named(NamedKey::ArrowLeft) => {
                self.selected = self.selected.saturating_sub(1);
            }
            Key::Named(NamedKey::Tab) if shift => {
                self.selected = self.selected.saturating_sub(1);
            }
            Key::Named(NamedKey::Enter) => {
                if let Some(it) = self.items.get(self.selected) {
                    let id = it.id.clone();
                    self.do_paste(&id, Some(shift));
                }
            }
            Key::Named(NamedKey::Delete) => {
                if let Some(it) = self.items.get(self.selected).cloned() {
                    let state_arc = self.state.clone();
        let mut st = state_arc.lock().unwrap();
                    st.store.remove(&it.id);
                    st.store.save();
                    st.bump();
                    self.refresh_items();
                }
            }
            Key::Named(NamedKey::Backspace) => {
                self.query.pop();
                self.refresh_items();
            }
            Key::Character(c) => {
                if ctrl {
                    match c.as_str() {
                        "c" => {
                            if let Some(it) = self.items.get(self.selected).cloned() {
                                self.state.lock().unwrap().copy_item(&it.id);
                                crate::sounds::play("copy");
                            }
                        }
                        "s" => {
                            if let Some(it) = self.items.get(self.selected).cloned() {
                                let state_arc = self.state.clone();
        let mut st = state_arc.lock().unwrap();
                                st.store.push_stack(&it.id);
                                st.store.save();
                                st.bump();
                            }
                        }
                        "f" => { /* search is always focused in this UI */ }
                        _ => {}
                    }
                } else {
                    match c.as_str() {
                        "e" => {
                            if let Some(it) = self.items.get(self.selected) {
                                self.edit = Some((it.id.clone(), it.content.clone()));
                            }
                        }
                        "s" if shift => self.stack_open = !self.stack_open,
                        _ => self.query.push_str(c),
                    }
                    self.refresh_items();
                }
            }
            _ => {}
        }
        self.request_redraw();
    }

    /* ---------- rendering ---------- */

    fn draw(&mut self) {
        let Some(window) = self.window.clone() else { return };
        let size = window.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));
        let mut pm = Pixmap::new(w, h).expect("pixmap");
        self.hits.clear();

        // glass panel background
        round_rect(&mut pm, 0.0, 0.0, w as f32, h as f32, 22.0, rgba(14, 16, 26, 245));
        self.layout_and_draw(&mut pm, w as f32, h as f32);

        let Some(surface) = self.surface.as_mut() else { return };
        if let (Some(nzw), Some(nzh)) = (
            std::num::NonZeroU32::new(w),
            std::num::NonZeroU32::new(h),
        ) {
            surface.resize(nzw, nzh).ok();
        }
        if let Ok(mut buf) = surface.buffer_mut() {
            let px = pm.pixels();
            for (dst, p) in buf.iter_mut().zip(px) {
                *dst = (0xFF << 24) | ((p.red() as u32) << 16) | ((p.green() as u32) << 8) | p.blue() as u32;
            }
            buf.present().ok();
        }
    }

    fn layout_and_draw(&mut self, pm: &mut Pixmap, _w: f32, _h: f32) {
        // sidebar
        self.text(pm, 30.0, 34.0, 13.0, "History", false, rgba(238, 240, 246, 235));
        self.hit(18.0, 20.0, 150.0, 28.0, Hit::SetFilter(0));
        if self.filter == 0 {
            round_rect(pm, 18.0, 18.0, 152.0, 32.0, 10.0, rgba(143, 176, 255, 46));
        }
        self.text(pm, 30.0, 66.0, 13.0, "Pinned", false, rgba(238, 240, 246, 190));
        self.hit(18.0, 52.0, 150.0, 28.0, Hit::SetFilter(1));
        if self.filter == 1 {
            round_rect(pm, 18.0, 50.0, 152.0, 32.0, 10.0, rgba(143, 176, 255, 46));
        }
        self.text(pm, 30.0, 100.0, 10.0, "PINBOARDS", false, rgba(238, 240, 246, 120));
        {
            let names: Vec<String> = self
                .state
                .lock()
                .unwrap()
                .store
                .pinboards
                .iter()
                .map(|p| p.name.clone())
                .collect();
            for (n, name) in names.iter().enumerate() {
                let y = 118.0 + n as f32 * 28.0;
                self.text(pm, 30.0, y + 6.0, 12.5, name, false, rgba(238, 240, 246, 190));
                self.hit(18.0, y - 6.0, 150.0, 26.0, Hit::SetFilter(n + 2));
                if self.filter == n + 2 {
                    round_rect(pm, 18.0, y - 8.0, 152.0, 30.0, 10.0, rgba(143, 176, 255, 46));
                }
            }
        }
        self.text(pm, 30.0, H - 30.0, 12.0, "+ New", false, rgba(238, 240, 246, 140));
        self.hit(18.0, H - 44.0, 150.0, 28.0, Hit::NewPinboard);
        line(pm, 182.0, 16.0, 182.0, H - 12.0, rgba(255, 255, 255, 22));

        if self.settings {
            self.draw_settings(pm);
            return;
        }

        // topbar: search + pills
        round_rect(pm, 196.0, 18.0, W - 196.0 - 230.0, 34.0, 17.0, rgba(255, 255, 255, 16));
        let q = if self.query.is_empty() { "Search history…".to_string() } else { self.query.clone() };
        self.text(pm, 212.0, 32.0, 13.0, &q, false, rgba(238, 240, 246, if self.query.is_empty() { 130 } else { 235 }));
        // stack chip
        {
            let stacked = self
                .state
                .lock()
                .unwrap()
                .store
                .items
                .iter()
                .filter(|i| i.stack_order.is_some())
                .count();
            if stacked > 0 {
                let label = format!("Stack x{stacked}");
                round_rect(pm, W - 224.0, 18.0, 92.0, 32.0, 16.0, rgba(143, 176, 255, 30));
                self.text(pm, W - 210.0, 31.0, 12.0, &label, true, rgba(143, 176, 255, 235));
                self.hit(W - 224.0, 18.0, 92.0, 32.0, Hit::StackToggle);
            }
        }
        round_rect(pm, W - 124.0, 18.0, 52.0, 32.0, 16.0, rgba(255, 255, 255, 16));
        self.text(pm, W - 112.0, 31.0, 12.0, "+ Pin", false, rgba(238, 240, 246, 200));
        self.hit(W - 124.0, 18.0, 52.0, 32.0, Hit::NewPinboard);
        round_rect(pm, W - 64.0, 18.0, 34.0, 32.0, 16.0, rgba(255, 255, 255, 16));
        self.text(pm, W - 55.0, 31.0, 12.0, "cfg", false, rgba(238, 240, 246, 200));
        self.hit(W - 64.0, 18.0, 34.0, 32.0, Hit::OpenSettings);

        // cards strip
        let mut x = 196.0;
        let cards = self.items.clone();
        for (i, item) in cards.iter().enumerate() {
            if x > W - 30.0 {
                break;
            }
            self.draw_card(pm, x, 64.0, 228.0, 252.0, i, item);
            x += 241.0;
        }
        if self.items.is_empty() {
            self.text(pm, 420.0, 170.0, 15.0, "Clipboard history is empty", true, rgba(238, 240, 246, 200));
            self.text(pm, 380.0, 196.0, 12.5, "Copy anything — it shows up here instantly.", false, rgba(238, 240, 246, 130));
        }

        // hints
        self.text(
            pm, 260.0, H - 22.0, 10.5,
            "<- -> navigate   Enter paste   Shift+Enter plain   E edit   Ctrl+C copy   Ctrl+S stack   Del delete   Esc close",
            false,
            rgba(238, 240, 246, 130),
        );

        if self.stack_open {
            self.draw_stack(pm);
        }
    }

    fn draw_card(&mut self, pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, idx: usize, item: &ClipItem) {
        let selected = idx == self.selected;
        round_rect(pm, x, y, w, h, 15.0, rgba(255, 255, 255, 14));
        if selected {
            stroke_round(pm, x, y, w, h, 15.0, rgba(143, 176, 255, 230), 1.5);
        }
        self.hit(x, y, w, h, Hit::Select(idx));
        // header buttons (appear area always reserved)
        self.text(pm, x + w - 66.0, y + 8.0, 11.0, if item.pinned { "*" } else { "o" }, false, rgba(238, 240, 246, 200));
        self.hit(x + w - 72.0, y + 2.0, 22.0, 22.0, Hit::Pin(idx));
        self.text(pm, x + w - 42.0, y + 8.0, 11.0, "ed", false, rgba(238, 240, 246, 200));
        self.hit(x + w - 48.0, y + 2.0, 22.0, 22.0, Hit::Edit(idx));
        self.text(pm, x + w - 20.0, y + 8.0, 11.0, "x", false, rgba(238, 240, 246, 200));
        self.hit(x + w - 26.0, y + 2.0, 22.0, 22.0, Hit::Delete(idx));

        let py = y + 30.0;
        let ph = h - 66.0;
        match item.item_type {
            ItemType::Color => {
                let c = parse_color(&item.content);
                round_rect(pm, x + 8.0, py, w - 16.0, ph, 10.0, c);
            }
            ItemType::Image => {
                if let Some(name) = &item.image_file {
                    if let Ok(png) = std::fs::read(self.state.lock().unwrap().store.image_dir.join(name)) {
                        if let Ok(img) = Pixmap::decode_png(&png) {
                            pm.draw_pixmap(
                                (x + 8.0) as i32,
                                py as i32,
                                img.as_ref(),
                                &tiny_skia::PixmapPaint::default(),
                                Transform::identity(),
                                None,
                            );
                        }
                    }
                }
            }
            ItemType::Link => {
                round_rect(pm, x + 16.0, py + 16.0, 44.0, 44.0, 11.0, rgba(109, 149, 255, 235));
                let letter: String = item.preview.chars().take(1).collect();
                self.text(pm, x + 32.0, py + 30.0, 18.0, &letter, true, rgba(255, 255, 255, 255));
                self.wrap(pm, x + 16.0, py + 76.0, w - 32.0, 13.0, &item.title, true);
                self.text(pm, x + 16.0, py + ph - 24.0, 11.5, &item.preview, false, rgba(238, 240, 246, 130));
            }
            ItemType::File => {
                let mut ly = py + 12.0;
                for f in item.content.lines().take(4) {
                    round_rect(pm, x + 12.0, ly, w - 24.0, 30.0, 8.0, rgba(255, 255, 255, 14));
                    let name = f.rsplit('/').next().unwrap_or(f);
                    self.text(pm, x + 22.0, ly + 9.0, 11.5, name, false, rgba(238, 240, 246, 210));
                    ly += 37.0;
                }
            }
            ItemType::Code => {
                round_rect(pm, x + 8.0, py, w - 16.0, ph, 10.0, rgba(0, 0, 0, 60));
                self.wrap_lines(pm, x + 16.0, py + 12.0, w - 32.0, 12.0, &item.content, true, 12);
            }
            ItemType::Text | ItemType::Html => {
                self.wrap_lines(pm, x + 14.0, py + 8.0, w - 28.0, 13.0, &item.content, false, 12);
            }
        }

        // meta row
        let badge = match item.item_type {
            ItemType::Text => "Text",
            ItemType::Link => "Link",
            ItemType::Code => "Code",
            ItemType::Color => "Color",
            ItemType::Image => "Image",
            ItemType::File => "Files",
            ItemType::Html => "Rich text",
        };
        round_rect(pm, x + 12.0, y + h - 28.0, 54.0, 18.0, 7.0, rgba(143, 176, 255, 38));
        self.text(pm, x + 18.0, y + h - 23.0, 9.5, badge, true, rgba(143, 176, 255, 235));
        if item.stack_order.is_some() {
            let label = format!("#{}", item.stack_order.unwrap_or(0));
            self.text(pm, x + 76.0, y + h - 23.0, 9.5, &label, true, rgba(143, 176, 255, 235));
        }
    }

    fn draw_stack(&mut self, pm: &mut Pixmap) {
        let state_arc = self.state.clone();
        let st = state_arc.lock().unwrap();
        let queued = st.store.stack_items();
        drop(st);
        let x = W - 360.0;
        let y = 60.0;
        round_rect(pm, x, y, 330.0, 240.0, 14.0, rgba(22, 25, 38, 250));
        self.text(pm, x + 14.0, y + 12.0, 12.0, &format!("Paste stack ({})", queued.len()), true, rgba(238, 240, 246, 235));
        self.text(pm, x + 240.0, y + 12.0, 11.0, "clear", false, rgba(238, 240, 246, 170));
        self.hit(x + 234.0, y + 4.0, 44.0, 22.0, Hit::StackClear);
        for (i, item) in queued.iter().enumerate().take(5) {
            let ry = y + 40.0 + i as f32 * 30.0;
            self.text(pm, x + 16.0, ry + 6.0, 11.0, &format!("{}", i + 1), true, rgba(143, 176, 255, 235));
            self.text(pm, x + 40.0, ry + 6.0, 11.5, &item.title, false, rgba(238, 240, 246, 210));
            self.text(pm, x + 280.0, ry + 6.0, 11.0, "x", false, rgba(238, 240, 246, 170));
            self.hit(x + 272.0, ry, 24.0, 24.0, Hit::StackRemove(i));
        }
        round_rect(pm, x + 12.0, y + 200.0, 306.0, 30.0, 9.0, rgba(109, 149, 255, 235));
        self.text(pm, x + 110.0, y + 208.0, 12.0, "Paste next", true, rgba(255, 255, 255, 255));
        self.hit(x + 12.0, y + 200.0, 306.0, 30.0, Hit::StackPasteNext);
    }

    fn draw_settings(&mut self, pm: &mut Pixmap) {
        let state_arc = self.state.clone();
        let st = state_arc.lock().unwrap();
        let s = st.store.settings.clone();
        drop(st);
        self.text(pm, 200.0, 30.0, 15.0, "ClipDeck Settings", true, rgba(238, 240, 246, 245));
        let rows: [(f32, &str, bool, Hit); 11] = [
            (56.0, "Watch clipboard", s.watch_enabled, Hit::Toggle("watch")),
            (86.0, "Play sound on copy", s.sound_copy, Hit::Toggle("sound_copy")),
            (116.0, "Play sound on paste", s.sound_paste, Hit::Toggle("sound_paste")),
            (146.0, "Ignore transient content", s.ignore_transient, Hit::Toggle("transient")),
            (176.0, "Ignore confidential content", s.ignore_confidential, Hit::Toggle("confidential")),
            (206.0, "Generate link previews", s.generate_link_previews, Hit::Toggle("link_previews")),
            (236.0, "Always paste as plain text", s.paste_as_plain_text, Hit::Toggle("plain")),
            (266.0, "Start ClipDeck at login", s.launch_at_login, Hit::Toggle("login")),
            (296.0, "Show overlay during screen sharing", s.show_in_screen_share, Hit::Toggle("share")),
            (326.0, "Paste stack: LIFO (off = FIFO)", s.stack_direction == "lifo", Hit::Toggle("stack_dir")),
            (356.0, "Global shortcut", false, Hit::ShortcutCapture),
        ];
        for (y, label, on, hit) in rows {
            self.text(pm, 210.0, y + 6.0, 12.5, label, false, rgba(238, 240, 246, 220));
            if matches!(hit, Hit::ShortcutCapture) {
                let shown = if self.capture { "Press keys… (Esc cancels)".to_string() } else { pretty_accel(&s.shortcut) };
                round_rect(pm, 640.0, y, 220.0, 26.0, 8.0, if self.capture { rgba(143, 176, 255, 40) } else { rgba(255, 255, 255, 18) });
                self.text(pm, 650.0, y + 6.0, 11.5, &shown, true, rgba(238, 240, 246, 235));
            } else {
                let (bx, bg) = if on { (838.0, rgba(109, 149, 255, 235)) } else { (812.0, rgba(255, 255, 255, 30)) };
                round_rect(pm, bx, y, 46.0, 26.0, 13.0, bg);
                round_rect(pm, if on { bx + 24.0 } else { bx + 4.0 }, y + 4.0, 18.0, 18.0, 9.0, rgba(255, 255, 255, 245));
            }
            self.hit(200.0, y - 2.0, 700.0, 28.0, hit);
        }
        round_rect(pm, 210.0, H - 60.0, 110.0, 30.0, 8.0, rgba(255, 255, 255, 18));
        self.text(pm, 232.0, H - 52.0, 12.0, "Export", true, rgba(238, 240, 246, 235));
        self.hit(210.0, H - 60.0, 110.0, 30.0, Hit::Export);
        round_rect(pm, 330.0, H - 60.0, 110.0, 30.0, 8.0, rgba(255, 255, 255, 18));
        self.text(pm, 352.0, H - 52.0, 12.0, "Import", true, rgba(238, 240, 246, 235));
        self.hit(330.0, H - 60.0, 110.0, 30.0, Hit::Import);
    }

    /* ---------- text primitives ---------- */

    fn hit(&mut self, x: f32, y: f32, w: f32, h: f32, action: Hit) {
        self.hits.push((x, y, w, h, action));
    }

    fn text(&self, pm: &mut Pixmap, x: f32, baseline: f32, px: f32, s: &str, mono: bool, color: [u8; 4]) {
        let font = if mono { &self.mono } else { &self.font };
        let mut pen = x;
        for ch in s.chars() {
            let (m, bitmap) = font.rasterize(ch, px);
            blit(pm, (pen + m.xmin as f32) as i32, (baseline + m.ymin as f32) as i32, m.width, m.height, &bitmap, color);
            pen += m.advance_width;
        }
    }

    fn wrap(&self, pm: &mut Pixmap, x: f32, y: f32, max_w: f32, px: f32, s: &str, mono: bool) {
        self.wrap_lines(pm, x, y, max_w, px, s, mono, 3);
    }

    fn wrap_lines(&self, pm: &mut Pixmap, x: f32, y: f32, max_w: f32, px: f32, s: &str, mono: bool, max_lines: usize) {
        let font = if mono { &self.mono } else { &self.font };
        let mut line = String::new();
        let mut ly = y;
        let mut lines = 0;
        for ch in s.chars() {
            let width: f32 = font
                .horizontal_line_metrics(px)
                .map(|m| m.new_line_size)
                .unwrap_or(px);
            let _ = width;
            let candidate = format!("{line}{ch}");
            let w: f32 = candidate
                .chars()
                .map(|c| font.metrics(c, px).advance_width)
                .sum();
            if w > max_w && !line.is_empty() {
                self.text(pm, x, ly + px, px, &line, mono, rgba(238, 240, 246, 220));
                ly += px * 1.55;
                lines += 1;
                line = ch.to_string();
                if lines >= max_lines {
                    return;
                }
            } else {
                line = candidate;
            }
        }
        if !line.is_empty() && lines < max_lines {
            self.text(pm, x, ly + px, px, &line, mono, rgba(238, 240, 246, 220));
        }
    }

}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> [u8; 4] {
    [r, g, b, a]
}

fn color_paint(c: [u8; 4]) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = true;
    p
}

/// Rounded rectangle built from four cubic corner arcs (kappa = 0.5522847498).
fn rounded_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
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

fn round_rect(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, c: [u8; 4]) {
    if let Some(path) = rounded_path(x, y, w, h, r) {
        pm.fill_path(&path, &color_paint(c), FillRule::Winding, Transform::identity(), None);
    }
}

fn stroke_round(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, c: [u8; 4], width: f32) {
    if let Some(path) = rounded_path(x, y, w, h, r) {
        pm.stroke_path(&path, &color_paint(c), &tiny_skia::Stroke { width, ..Default::default() }, Transform::identity(), None);
    }
}

fn line(pm: &mut Pixmap, x1: f32, y1: f32, x2: f32, y2: f32, c: [u8; 4]) {
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
            let a = cov[(row as usize * w + col as usize)] as u32;
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

fn parse_color(s: &str) -> [u8; 4] {
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

fn accel_from_key_event(ev: &KeyEvent, mods: ModifiersState) -> Option<String> {
    if matches!(ev.logical_key, Key::Named(NamedKey::Escape)) {
        return None;
    }
    let code = key_code(&ev.logical_key)?;
    let mut kp = KeyPress {
        ctrl: mods.contains(ModifiersState::CONTROL),
        meta: mods.contains(ModifiersState::SUPER),
        alt: mods.contains(ModifiersState::ALT),
        shift: mods.contains(ModifiersState::SHIFT),
        code,
    };
    if !kp.ctrl && !kp.meta && !kp.alt && !kp.shift {
        return None;
    }
    let out = record_accel(&kp);
    let _ = &mut kp;
    out
}

fn key_code(key: &Key) -> Option<String> {
    Some(match key {
        Key::Character(c) => {
            let c = c.chars().next()?;
            if c.is_ascii_alphabetic() {
                format!("Key{}", c.to_ascii_uppercase())
            } else if c.is_ascii_digit() {
                format!("Digit{c}")
            } else {
                match c {
                    ' ' => "Space".into(),
                    ',' => "Comma".into(),
                    '.' => "Period".into(),
                    '/' => "Slash".into(),
                    '\\' => "Backslash".into(),
                    '-' => "Minus".into(),
                    '=' => "Equal".into(),
                    ';' => "Semicolon".into(),
                    '\'' => "Quote".into(),
                    '`' => "Backquote".into(),
                    '[' => "BracketLeft".into(),
                    ']' => "BracketRight".into(),
                    _ => return None,
                }
            }
        }
        Key::Named(n) => match n {
            NamedKey::Enter => "Enter".into(),
            NamedKey::Tab => "Tab".into(),
            NamedKey::Backspace => "Backspace".into(),
            NamedKey::Delete => "Delete".into(),
            NamedKey::ArrowUp => "ArrowUp".into(),
            NamedKey::ArrowDown => "ArrowDown".into(),
            NamedKey::ArrowLeft => "ArrowLeft".into(),
            NamedKey::ArrowRight => "ArrowRight".into(),
            NamedKey::Space => "Space".into(),
            NamedKey::F1 => "F1".into(), NamedKey::F2 => "F2".into(),
            NamedKey::F3 => "F3".into(), NamedKey::F4 => "F4".into(),
            NamedKey::F5 => "F5".into(), NamedKey::F6 => "F6".into(),
            NamedKey::F7 => "F7".into(), NamedKey::F8 => "F8".into(),
            NamedKey::F9 => "F9".into(), NamedKey::F10 => "F10".into(),
            NamedKey::F11 => "F11".into(), NamedKey::F12 => "F12".into(),
            _ => return None,
        },
        _ => return None,
    })
}
