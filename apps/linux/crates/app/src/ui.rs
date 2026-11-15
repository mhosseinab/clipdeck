//! Overlay UI: window, rendering, interaction.
//!
//! Architecture (SOLID / KISS / DRY):
//! - `AppState` is OWNED by this main-thread struct — no locks anywhere.
//! - EVERY state mutation flows through `apply(AppEvent)`, the single
//!   dispatcher (single point of truth for events); producers on other
//!   threads reach it through the shared `EventLoopProxy`.
//! - Side effects (clipboard writes, hooks, sounds, window visibility) come
//!   back as `Effects` bundles and are executed by one `run_effects` method.
//! - View state (query/selection/scroll/…) is UI-local and never shared.

use crate::draw::*;
use crate::events::{AppEvent, WritePurpose};
use crate::state::{AppState, Effects};
use crate::presentation::Presenter;
use crate::tray::{self, TrayView};
use crate::worker::WorkerCmd;
use clipdeck_core::{pretty_accel, record_accel, ClipItem, KeyPress, Pinboard};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};
use tiny_skia::{Color, Pixmap};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent, StartCause};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

const W: f32 = 1240.0;
const H: f32 = 372.0;

const KEYBOARD_HINTS: [(&str, &str); 9] = [("Left/Right", "Select"), ("Enter", "Copy"), ("Shift+Enter", "Plain"),
            ("Double-click", "Copy"), ("Ctrl+E", "Edit"), ("Ctrl+C", "Copy"),
            ("Ctrl+S", "Stack"), ("Del", "Remove"), ("Esc", "Close")];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hit {
    Select(usize),
    Paste(usize),
    Copy(usize),
    Delete(usize),
    Pin(usize),
    Edit(usize),
    OpenSettings,
    OpenKeyboardSettings,
    CopyShortcutCommand,
    CloseWindow,
    StackToggle,
    StackPasteNext,
    StackClear,
    StackRemove(usize),
    SaveEdit,
    CancelEdit,
    SetFilter(usize),
    NewPinboard,
    DeletePinboard(usize),
    RenamePinboard(usize),
    MoveToBoard(usize),
    ChoosePinboard(usize),
    SaveBoardName,
    CancelDialog,
    DialogBackground,
    ConfirmDelete,
    ToggleDeleteHistory,
    Toggle(&'static str),
    ShortcutCapture,
    Export,
    Import,
}

#[derive(Debug, Clone)]
enum Dialog {
    Rename { id: String, draft: String, select_all: bool },
    Move { item_id: String, scroll: usize },
    ConfirmDelete { target: DeleteTarget },
}

#[derive(Debug, Clone)]
enum DeleteTarget {
    Clip { id: String, label: String, board_id: Option<String>, delete_history: bool },
    Board { id: String, label: String },
}

#[derive(Debug, Clone)]
struct CardDrag {
    id: String,
    start: (f32, f32),
    active: bool,
    board_id: Option<String>,
}

#[derive(Debug, Clone)]
struct DropFeedback {
    board_id: String,
    started: std::time::Instant,
}

impl DropFeedback {
    fn strength(&self, now: std::time::Instant) -> f32 {
        let progress = now.saturating_duration_since(self.started).as_secs_f32() / 0.24;
        (1.0 - progress).clamp(0.0, 1.0).powi(2)
    }

    fn opacity(&self, now: std::time::Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.started).as_secs_f32();
        ((0.8 - elapsed) / 0.16).clamp(0.0, 1.0)
    }

    fn expired(&self, now: std::time::Instant) -> bool {
        now.saturating_duration_since(self.started) >= std::time::Duration::from_millis(800)
    }
}

pub struct Ui {
    app: AppState,
    worker_tx: Sender<WorkerCmd>,
    accel_tx: Sender<String>,
    tray_view: Arc<RwLock<TrayView>>,
    fonts: Fonts,
    thumbnails: ThumbnailCache,
    window: Option<Arc<Window>>,
    surface: Option<Presenter>,
    // view state
    query: String,
    selected: usize,
    filter: usize,
    editing: Option<(String, String)>,
    dialog: Option<Dialog>,
    drag: Option<CardDrag>,
    drop_feedback: Option<DropFeedback>,
    settings: bool,
    settings_notice: Option<String>,
    capture: bool,
    wayland_session: bool,
    stack_open: bool,
    scroll_x: f32,
    view_w: f32,
    visible: bool,
    mods: ModifiersState,
    mouse: Option<(f32, f32)>,
    last_card_click: Option<(String, std::time::Instant, (f32, f32))>,
    hits: Vec<(f32, f32, f32, f32, Hit)>,
    items: Vec<ClipItem>,
    pinboards: Vec<Pinboard>,
}

pub fn run(
    app: AppState,
    _events: EventLoopProxy<AppEvent>,
    worker_tx: Sender<WorkerCmd>,
    accel_tx: Sender<String>,
    tray_view: Arc<RwLock<TrayView>>,
    event_loop: EventLoop<AppEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut ui = Ui {
        app,
        worker_tx,
        accel_tx,
        tray_view,
        fonts: Fonts::bundled(),
        thumbnails: ThumbnailCache::default(),
        window: None,
        surface: None,
        query: String::new(),
        selected: 0,
        filter: 0,
        editing: None,
        dialog: None,
        drag: None,
        drop_feedback: None,
        settings: false,
        settings_notice: None,
        capture: false,
        wayland_session: std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE").as_deref() == Ok("wayland"),
        stack_open: false,
        // Frame-dump QA can select a scroll offset without injected input.
        scroll_x: if std::env::var_os("CLIPDECK_DUMP_UI").is_some() {
            std::env::var("CLIPDECK_DUMP_SCROLL_X").ok().and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(0.0)
        } else { 0.0 },
        view_w: W,
        visible: false,
        mods: ModifiersState::empty(),
        mouse: None,
        last_card_click: None,
        hits: Vec::new(),
        items: Vec::new(),
        pinboards: Vec::new(),
    };
    event_loop.run_app(&mut ui)?;
    Ok(())
}

/* ===================== event plumbing ===================== */

impl Ui {
    /// The single point of truth: every state mutation and side effect.
    fn apply(&mut self, event: AppEvent) {
        let effects = match event {
            AppEvent::Captured { item, source_app } => self.app.record_captured(item, source_app),
            AppEvent::Written { purpose } => {
                // Clipboard completion never launches keyboard/input tools.
                match purpose {
                    WritePurpose::Paste | WritePurpose::StackPaste => {
                        let mut fx = Effects::none();
                        fx.sound = Some("paste");
                        fx
                    }
                    WritePurpose::Copy | WritePurpose::TrayCopy => Effects::none(),
                }
            }
            AppEvent::Show => {
                self.show_overlay();
                Effects::none()
            }
            AppEvent::Hide => {
                self.hide_overlay();
                Effects::none()
            }
            AppEvent::Toggle => {
                if self.visible {
                    self.hide_overlay();
                } else {
                    self.show_overlay();
                }
                Effects::none()
            }
            AppEvent::OpenSettings => {
                self.show_overlay();
                self.settings = true;
                self.dialog = None;
                Effects::none()
            }
            AppEvent::OpenKeyboardSettings => {
                let mut effects = Effects::none();
                effects.open_keyboard_settings = true;
                effects
            }
            AppEvent::CopyShortcutCommand => {
                let mut effects = Effects::none();
                effects.writes.push((crate::events::WritePayload::Text(shortcut_command()), WritePurpose::Copy));
                self.settings_notice = Some("Paste this into the custom shortcut's Command field.".into());
                effects
            }
            AppEvent::Quit => {
                self.app.save();
                std::process::exit(0);
            }
            AppEvent::ToggleWatch => self.app.toggle_watch(),
            AppEvent::Copy { id, purpose } => self.app.copy(&id, purpose),
            AppEvent::Paste { id, plain } => self.app.paste(&id, plain),
            AppEvent::Activate { id, plain } => self.app.activate(&id, plain),
            AppEvent::StackPush { id } => self.app.stack_push(&id),
            AppEvent::StackPop => self.app.stack_pop(),
            AppEvent::StackClear => self.app.stack_clear(),
            AppEvent::StackUnstack { id } => self.app.stack_unstack(&id),
            AppEvent::Delete { id } => self.app.delete(&id),
            AppEvent::Pin { id, pinned } => self.app.pin(&id, pinned),
            AppEvent::NewPinboard => {
                let effects = self.app.add_pinboard();
                self.filter = self.app.store.pinboards.len() + 1;
                self.query.clear();
                self.selected = 0;
                self.scroll_x = 0.0;
                effects
            }
            AppEvent::DeletePinboard { id } => {
                let active_board = self.filter.checked_sub(2)
                    .and_then(|i| self.pinboards.get(i)).map(|p| p.id.clone());
                let effects = self.app.delete_pinboard(&id);
                if self.filter >= 2 {
                    self.filter = active_board.and_then(|active| self.app.store.pinboards.iter()
                        .position(|board| board.id == active)).map(|i| i + 2).unwrap_or(0);
                }
                self.selected = 0;
                self.scroll_x = 0.0;
                effects
            }
            AppEvent::RenamePinboard { id, name } => {
                self.dialog = None;
                self.app.rename_pinboard(&id, &name)
            }
            AppEvent::MoveToPinboard { id, pinboard_id } => {
                self.dialog = None;
                self.app.move_to_pinboard(&id, pinboard_id.as_deref())
            }
            AppEvent::SaveEdit { id, content } => {
                self.editing = None;
                self.app.save_edit(&id, &content)
            }
            AppEvent::SetShortcut { accel } => {
                self.app.store.settings.shortcut = accel.clone();
                self.accel_tx.send(accel).ok();
                Effects::none()
            }
            AppEvent::ToggleSetting { key } => {
                if self.setting_unavailable_reason(key).is_some() { Effects::none() }
                else { self.app.toggle_setting(key) }
            },
            AppEvent::Export { path } => {
                self.app.export_to(&path);
                Effects::none()
            }
            AppEvent::Import { path } => self.app.import_from(&path),
            AppEvent::Refresh => Effects::none(),
        };
        self.app.save();
        self.run_effects(effects);
    }

    /// Execute one Effects bundle: the only place side effects happen.
    fn run_effects(&mut self, fx: Effects) {
        if let Some(watching) = fx.watching {
            self.worker_tx.send(WorkerCmd::SetWatching(watching)).ok();
        }
        if fx.open_keyboard_settings {
            match keyboard_settings_command().spawn() {
                Ok(mut child) => {
                    std::thread::spawn(move || { let _ = child.wait(); });
                    self.settings_notice = None;
                    self.hide_overlay(); // The always-on-top overlay must not cover GNOME Settings.
                }
                Err(error) => {
                    self.settings_notice = Some(format!("Could not open Keyboard Settings: {error}"));
                }
            }
        }
        for (payload, purpose) in fx.writes {
            self.worker_tx.send(WorkerCmd::Write { payload, purpose }).ok();
        }
        for (event, content) in fx.hooks {
            self.app.run_hooks_for(event, &content);
        }
        if let Some(kind) = fx.sound {
            crate::sounds::play(kind);
        }
        if fx.hidden {
            self.hide_overlay();
        }
        self.refresh();
        self.request_redraw();
    }

    fn refresh(&mut self) {
        self.pinboards = self.app.store.pinboards.clone();
        let pin_ids: Vec<String> = self.pinboards.iter().map(|p| p.id.clone()).collect();
        self.items = self.app.items(&self.query, self.filter, &pin_ids);
        if self.selected >= self.items.len() {
            self.selected = 0;
        }
        let watching = self.app.settings().watch_enabled;
        tray::refresh_view(&self.tray_view, watching, &self.app.store.items);
    }

    fn show_overlay(&mut self) {
        self.settings = false;
        if let Some(w) = &self.window {
            w.set_visible(true);
            w.focus_window();
        }
        self.visible = true;
        self.refresh();
        if std::env::var_os("CLIPDECK_DUMP_UI").is_some() && self.dialog.is_none() {
            self.dialog = match std::env::var("CLIPDECK_DUMP_BOARD_DIALOG").as_deref() {
                Ok("rename") => self.pinboards.first().map(|board| Dialog::Rename {
                    id: board.id.clone(), draft: board.name.clone(), select_all: true,
                }),
                Ok("move") => self.items.first().map(|item| Dialog::Move {
                    item_id: item.id.clone(), scroll: 0,
                }),
                Ok("delete") => self.items.first().map(|item| Dialog::ConfirmDelete { target: DeleteTarget::Clip {
                    id: item.id.clone(), label: item.title.clone(), board_id: None, delete_history: false,
                } }),
                Ok("remove-board") => self.items.first().map(|item| Dialog::ConfirmDelete { target: DeleteTarget::Clip {
                    id: item.id.clone(), label: item.title.clone(), board_id: item.pinboard_id.clone(), delete_history: false,
                } }),
                Ok("delete-board") => self.pinboards.first().map(|board| Dialog::ConfirmDelete { target: DeleteTarget::Board {
                    id: board.id.clone(), label: board.name.clone(),
                } }),
                _ => None,
            };
            if self.drop_feedback.is_none() && std::env::var("CLIPDECK_DUMP_BOARD_DIALOG").as_deref() == Ok("drop") {
                self.drop_feedback = self.pinboards.first().map(|board| DropFeedback { board_id: board.id.clone(), started: std::time::Instant::now() });
            }
        }
    }

    fn hide_overlay(&mut self) {
        if self.wayland_session {
            // set_visible(false) is a no-op on Wayland: release the surface first,
            // then the window. The next Show recreates both on the event loop.
            self.surface = None;
            self.window = None;
        } else if let Some(window) = &self.window {
            window.set_visible(false);
        }
        self.visible = false;
        self.settings = false;
        self.capture = false;
        self.editing = None;
        self.dialog = None;
        self.drag = None;
        self.drop_feedback = None;
        self.stack_open = false;
        self.mouse = None;
        self.last_card_click = None;
        self.hits.clear();
    }

    fn request_redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}

/* ===================== winit wiring ===================== */

fn window_attributes() -> winit::window::WindowAttributes {
    use winit::platform::wayland::WindowAttributesExtWayland;
    let (width, height, rgba) = crate::worker::decode_png(include_bytes!("../assets/icon.png"))
        .expect("bundled application PNG");
    let icon = winit::window::Icon::from_rgba(rgba, width as u32, height as u32)
        .expect("bundled application icon dimensions");
    Window::default_attributes()
            .with_name("clipdeck", "ClipDeck")
            .with_window_icon(Some(icon))
            .with_title("ClipDeck")
            .with_window_level(winit::window::WindowLevel::AlwaysOnTop)
            .with_inner_size(winit::dpi::LogicalSize::new(W, H))
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
}

impl Ui {
    fn ensure_window(&mut self, el: &ActiveEventLoop) {
        if !self.visible || self.window.is_some() {
            return;
        }
        let attrs = window_attributes();
        let window = Arc::new(el.create_window(attrs).expect("window"));
        let surface = Presenter::new(window.clone()).expect("window presentation");
        self.window = Some(window);
        self.surface = Some(surface);
        self.refresh();
        self.request_redraw();
    }
}

impl ApplicationHandler<AppEvent> for Ui {
    fn new_events(&mut self, _el: &ActiveEventLoop, cause: StartCause) {
        let now = std::time::Instant::now();
        if self.drop_feedback.as_ref().is_some_and(|feedback| feedback.expired(now))
            || (self.drop_feedback.is_some() && matches!(cause, StartCause::ResumeTimeReached { .. })) {
            self.advance_drop_animation(now);
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let flow = self.next_animation_deadline(std::time::Instant::now())
            .map(ControlFlow::WaitUntil).unwrap_or(ControlFlow::Wait);
        el.set_control_flow(flow);
    }

    fn resumed(&mut self, el: &ActiveEventLoop) {
        self.ensure_window(el);
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: AppEvent) {
        self.apply(event);
        self.ensure_window(el);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.window.as_ref().map(|window| window.id()) != Some(id) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => self.apply(AppEvent::Hide),
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::Resized(_) => self.request_redraw(),
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer_moved(position.x as f32, position.y as f32);
            }
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => self.pointer_pressed(),
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => self.pointer_released(),
            WindowEvent::CursorLeft { .. } | WindowEvent::Focused(false) => {
                self.drag = None;
                self.last_card_click = None;
                self.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y * 60.0,
                    winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32,
                };
                if self.dialog.is_some() {
                    let height = self.window.as_ref().map(|w| w.inner_size().height as f32).unwrap_or(H);
                    self.scroll_board_picker(dy, height);
                } else {
                    self.scroll_x = (self.scroll_x - dy).max(0.0);
                }
                self.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => self.key(event),
            _ => {}
        }
        self.ensure_window(el);
    }
}


/* ===================== interaction ===================== */

impl Ui {
    fn key(&mut self, ev: KeyEvent) {
        if ev.state != ElementState::Pressed {
            return;
        }
        let ctrl = self.mods.contains(ModifiersState::CONTROL);
        let shift = self.mods.contains(ModifiersState::SHIFT);

        if ev.logical_key == Key::Named(NamedKey::Escape) {
            self.handle_key_action(KeyAction::Close);
            return;
        }
        if self.dialog.is_some() {
            self.handle_key_action(route_key(&ev.logical_key, ctrl, shift));
            return;
        }
        if self.capture {
            if let Key::Named(NamedKey::Escape) = ev.logical_key {
                self.capture = false;
                self.request_redraw();
                return;
            }
            if let Some(accel) = accel_from_key_event(&ev, self.mods) {
                self.capture = false;
                self.apply(AppEvent::SetShortcut { accel });
            }
            self.request_redraw();
            return;
        }

        if self.editing.is_some() {
            match &ev.logical_key {
                Key::Named(NamedKey::Escape) => {
                    self.editing = None;
                    self.request_redraw();
                }
                Key::Named(NamedKey::Enter) => {
                    if let Some((id, content)) = self.editing.clone() {
                        self.apply(AppEvent::SaveEdit { id, content });
                    }
                }
                Key::Named(NamedKey::Backspace) => {
                    if let Some((_, draft)) = &mut self.editing {
                        draft.pop();
                    }
                    self.request_redraw();
                }
                Key::Character(c) => {
                    if let Some((_, draft)) = &mut self.editing {
                        draft.push_str(c);
                    }
                    self.request_redraw();
                }
                _ => {}
            }
            return;
        }

        let action = route_key(&ev.logical_key, ctrl, shift);
        if action == KeyAction::Ignore && !ctrl {
            if let Some(text) = &ev.text {
                let text: String = text.chars().filter(|c| !c.is_control()).collect();
                if !text.is_empty() {
                    self.query.push_str(&text);
                    self.refresh();
                    self.request_redraw();
                }
            }
        }
        self.handle_key_action(action);
    }

    fn handle_key_action(&mut self, action: KeyAction) {
        if self.dialog.is_some() && action != KeyAction::Close {
            self.handle_dialog_key(action);
            return;
        }
        match action {
            KeyAction::Close => self.apply(AppEvent::Hide),
            KeyAction::Next => {
                self.selected = (self.selected + 1).min(self.items.len().saturating_sub(1));
                self.scroll_selected_into_view();
                self.request_redraw();
            }
            KeyAction::Prev => {
                self.selected = self.selected.saturating_sub(1);
                self.scroll_selected_into_view();
                self.request_redraw();
            }
            KeyAction::Activate { plain } => {
                if let Some(it) = self.items.get(self.selected) {
                    self.apply(AppEvent::Activate { id: it.id.clone(), plain });
                }
            }
            KeyAction::DeleteSelected => self.request_delete_clip(self.selected),
            KeyAction::CopySelected => {
                if let Some(it) = self.items.get(self.selected) {
                    self.apply(AppEvent::Copy { id: it.id.clone(), purpose: WritePurpose::Copy });
                }
            }
            KeyAction::StackSelected => {
                if let Some(it) = self.items.get(self.selected) {
                    self.apply(AppEvent::StackPush { id: it.id.clone() });
                }
            }
            KeyAction::EditSelected => {
                if let Some(it) = self.items.get(self.selected) {
                    self.editing = Some((it.id.clone(), it.content.clone()));
                    self.request_redraw();
                }
            }
            KeyAction::QueryPop => {
                self.query.pop();
                self.refresh();
                self.request_redraw();
            }
            KeyAction::QueryPush(s) => {
                self.query.push_str(&s);
                self.refresh();
                self.request_redraw();
            }
            KeyAction::Ignore | KeyAction::SelectAll => {}
        }
    }

    fn next_animation_deadline(&self, now: std::time::Instant) -> Option<std::time::Instant> {
        let feedback = self.drop_feedback.as_ref()?;
        if !self.visible || feedback.expired(now) { return None; }
        let elapsed = now.saturating_duration_since(feedback.started).as_millis() as u64;
        let next_ms = if elapsed < 240 { ((elapsed / 16 + 1) * 16).min(240) }
            else if elapsed < 640 { 640 }
            else { ((elapsed / 16 + 1) * 16).min(800) };
        Some(feedback.started + std::time::Duration::from_millis(next_ms))
    }

    fn advance_drop_animation(&mut self, now: std::time::Instant) {
        if self.drop_feedback.as_ref().is_some_and(|feedback| feedback.expired(now)) {
            self.drop_feedback = None;
        }
        self.request_redraw();
    }

    fn hit_at(&self, x: f32, y: f32) -> Option<Hit> {
        self.hits.iter().rev().find(|(hx, hy, hw, hh, _)| {
            *hw > 0.0 && *hh > 0.0 && x >= *hx && x < hx + hw && y >= *hy && y < hy + hh
        }).map(|(_, _, _, _, hit)| *hit)
    }

    fn pointer_pressed(&mut self) {
        let Some((x, y)) = self.mouse else { return };
        self.drag = None;
        if self.dialog.is_none() && self.editing.is_none() && !self.settings {
            if let Some(Hit::Select(i)) = self.hit_at(x, y) {
                if let Some(item) = self.items.get(i) {
                    self.selected = i;
                    self.drag = Some(CardDrag { id: item.id.clone(), start: (x, y), active: false, board_id: None });
                    self.request_redraw();
                    return;
                }
            }
        }
        self.click(); // Existing buttons keep their press behavior.
    }

    fn sidebar_board_at(&self, x: f32, y: f32) -> Option<String> {
        let height = self.window.as_ref().map(|window| window.inner_size().height as f32).unwrap_or(H);
        if !(18.0..170.0).contains(&x) || y >= height - 48.0 { return None; }
        self.pinboards.iter().enumerate().find(|(n, _)| {
            let top = 110.0 + *n as f32 * 28.0;
            y >= top && y < top + 28.0
        }).map(|(_, board)| board.id.clone())
    }

    fn pointer_moved(&mut self, x: f32, y: f32) {
        self.mouse = Some((x, y));
        let target = self.sidebar_board_at(x, y);
        if let Some(drag) = &mut self.drag {
            let dx = x - drag.start.0;
            let dy = y - drag.start.1;
            drag.active |= dx * dx + dy * dy >= 64.0;
            if drag.active {
                drag.board_id = target;
                self.last_card_click = None;
                self.request_redraw();
            }
        }
    }

    fn pointer_released(&mut self) {
        let Some(drag) = self.drag.take() else { return };
        if drag.active {
            self.last_card_click = None;
            if let Some((x, y)) = self.mouse {
                if let Some(board_id) = self.sidebar_board_at(x, y) {
                    let old_board = self.app.store.find(&drag.id).and_then(|item| item.pinboard_id.clone());
                    self.apply(AppEvent::MoveToPinboard { id: drag.id.clone(), pinboard_id: Some(board_id.clone()) });
                    if old_board.as_deref() != Some(board_id.as_str()) && self.app.store.find(&drag.id)
                        .is_some_and(|item| item.pinboard_id.as_deref() == Some(board_id.as_str())) {
                        self.drop_feedback = Some(DropFeedback { board_id, started: std::time::Instant::now() });
                    }
                }
            }
        } else if let Some((x, y)) = self.mouse {
            self.select_card(&drag.id, x, y);
        }
        self.request_redraw();
    }

    fn select_card(&mut self, id: &str, x: f32, y: f32) {
        let Some(index) = self.items.iter().position(|item| item.id == id) else { return };
        self.selected = index;
        let now = std::time::Instant::now();
        let double_click = self.last_card_click.as_ref().is_some_and(|(previous, when, (px, py))| {
            previous == id && now.duration_since(*when) <= std::time::Duration::from_millis(400)
                && (x - px).abs() <= 4.0 && (y - py).abs() <= 4.0
        });
        if double_click {
            self.apply(AppEvent::Activate { id: id.to_owned(), plain: false });
        } else {
            self.last_card_click = Some((id.to_owned(), now, (x, y)));
        }
        self.request_redraw();
    }

    fn click(&mut self) {
        let Some((x, y)) = self.mouse else { return };
        let hit = self.hit_at(x, y);
        let Some(hit) = hit else { self.last_card_click = None; return };
        if !matches!(hit, Hit::Select(_)) {
            self.last_card_click = None;
        }
        match hit {
            Hit::Select(i) => {
                if let Some(id) = self.items.get(i).map(|item| item.id.clone()) {
                    self.select_card(&id, x, y);
                }
            }
            Hit::Paste(i) => {
                if let Some(it) = self.items.get(i) {
                    self.apply(AppEvent::Paste { id: it.id.clone(), plain: None });
                }
            }
            Hit::Copy(i) => {
                if let Some(it) = self.items.get(i) {
                    self.apply(AppEvent::Copy { id: it.id.clone(), purpose: WritePurpose::Copy });
                }
            }
            Hit::Delete(i) => self.request_delete_clip(i),
            Hit::Pin(i) => {
                if let Some(it) = self.items.get(i) {
                    self.apply(AppEvent::Pin { id: it.id.clone(), pinned: !it.pinned });
                }
            }
            Hit::Edit(i) => {
                if let Some(it) = self.items.get(i) {
                    self.editing = Some((it.id.clone(), it.content.clone()));
                    self.request_redraw();
                }
            }
            Hit::OpenKeyboardSettings => self.apply(AppEvent::OpenKeyboardSettings),
            Hit::CopyShortcutCommand => self.apply(AppEvent::CopyShortcutCommand),
            Hit::OpenSettings => {
                self.settings = true;
                self.request_redraw();
            }
            Hit::CloseWindow => self.apply(AppEvent::Hide),
            Hit::StackToggle => {
                self.stack_open = !self.stack_open;
                self.request_redraw();
            }
            Hit::StackPasteNext => self.apply(AppEvent::StackPop),
            Hit::StackClear => self.apply(AppEvent::StackClear),
            Hit::StackRemove(i) => {
                let queued = self.app.store.stack_items();
                if let Some(it) = queued.get(i) {
                    self.apply(AppEvent::StackUnstack { id: it.id.clone() });
                }
            }
            Hit::SaveEdit => {
                if let Some((id, content)) = self.editing.clone() {
                    self.apply(AppEvent::SaveEdit { id, content });
                }
            }
            Hit::CancelEdit => {
                self.editing = None;
                self.request_redraw();
            }
            Hit::RenamePinboard(i) => {
                if let Some(board) = self.pinboards.get(i) {
                    self.dialog = Some(Dialog::Rename {
                        id: board.id.clone(), draft: board.name.clone(), select_all: true,
                    });
                    self.settings = false;
                    self.editing = None;
                    self.capture = false;
                    self.request_redraw();
                }
            }
            Hit::MoveToBoard(i) => {
                if let Some(item) = self.items.get(i) {
                    self.selected = i;
                    self.dialog = Some(Dialog::Move { item_id: item.id.clone(), scroll: 0 });
                    self.editing = None;
                    self.request_redraw();
                }
            }
            Hit::ChoosePinboard(n) => {
                if let Some(Dialog::Move { item_id, .. }) = self.dialog.clone() {
                    let pinboard_id = if n == 0 { None } else {
                        let Some(board) = self.pinboards.get(n - 1) else { return };
                        Some(board.id.clone())
                    };
                    self.apply(AppEvent::MoveToPinboard { id: item_id, pinboard_id });
                }
            }
            Hit::SaveBoardName => self.save_board_name(),
            Hit::CancelDialog => {
                self.dialog = None;
                self.request_redraw();
            }
            Hit::DialogBackground => {}
            Hit::ConfirmDelete => self.confirm_delete(),
            Hit::ToggleDeleteHistory => {
                if let Some(Dialog::ConfirmDelete { target: DeleteTarget::Clip { board_id: Some(_), delete_history, .. } }) = &mut self.dialog {
                    *delete_history = !*delete_history;
                    self.request_redraw();
                }
            }
            Hit::NewPinboard => self.apply(AppEvent::NewPinboard),
            Hit::DeletePinboard(i) => {
                if let Some(board) = self.pinboards.get(i) {
                    self.dialog = Some(Dialog::ConfirmDelete { target: DeleteTarget::Board {
                        id: board.id.clone(), label: board.name.clone(),
                    } });
                    self.settings = false;
                    self.request_redraw();
                }
            }
            Hit::SetFilter(n) => {
                self.filter = n;
                self.selected = 0;
                self.refresh();
                self.request_redraw();
            }
            Hit::Toggle(key) => self.apply(AppEvent::ToggleSetting { key }),
            Hit::ShortcutCapture => {
                self.capture = true;
                self.request_redraw();
            }
            Hit::Export => self.apply(AppEvent::Export { path: "clipdeck-export.json".into() }),
            Hit::Import => self.apply(AppEvent::Import { path: "clipdeck-export.json".into() }),
        }
    }

    fn request_delete_clip(&mut self, index: usize) {
        if let Some(item) = self.items.get(index) {
            let board_id = self.filter.checked_sub(2).and_then(|i| self.pinboards.get(i))
                .filter(|board| item.pinboard_id.as_deref() == Some(board.id.as_str()))
                .map(|board| board.id.clone());
            self.dialog = Some(Dialog::ConfirmDelete { target: DeleteTarget::Clip {
                id: item.id.clone(), label: item.title.clone(), board_id, delete_history: false,
            } });
            self.editing = None;
            self.drag = None;
            self.request_redraw();
        }
    }

    fn confirm_delete(&mut self) {
        if let Some(Dialog::ConfirmDelete { target }) = self.dialog.take() {
            match target {
                DeleteTarget::Clip { id, board_id: Some(board), delete_history: false, .. } => {
                    if self.app.store.find(&id).is_some_and(|item| item.pinboard_id.as_deref() == Some(board.as_str())) {
                        self.apply(AppEvent::MoveToPinboard { id, pinboard_id: None });
                    }
                }
                DeleteTarget::Clip { id, .. } => self.apply(AppEvent::Delete { id }),
                DeleteTarget::Board { id, .. } => self.apply(AppEvent::DeletePinboard { id }),
            }
        }
    }

    fn save_board_name(&mut self) {
        if let Some(Dialog::Rename { id, draft, .. }) = self.dialog.clone() {
            if !draft.trim().is_empty() {
                self.apply(AppEvent::RenamePinboard { id, name: draft });
            }
        }
    }

    fn handle_dialog_key(&mut self, action: KeyAction) {
        if matches!(action, KeyAction::Activate { .. }) {
            self.save_board_name();
            return;
        }
        if let Some(Dialog::Rename { draft, select_all, .. }) = &mut self.dialog {
            match action {
                KeyAction::SelectAll => *select_all = true,
                KeyAction::QueryPush(text) => {
                    if *select_all { draft.clear(); }
                    *select_all = false;
                    draft.extend(text.chars().filter(|c| !c.is_control()));
                }
                KeyAction::QueryPop => {
                    if *select_all { draft.clear(); } else { draft.pop(); }
                    *select_all = false;
                }
                _ => {}
            }
            self.request_redraw();
        }
    }

    fn scroll_board_picker(&mut self, delta: f32, height: f32) {
        if let Some(Dialog::Move { scroll, .. }) = &mut self.dialog {
            let max = (self.pinboards.len() + 1).saturating_sub(board_picker_rows(height));
            if delta < 0.0 { *scroll = (*scroll + 1).min(max); }
            if delta > 0.0 { *scroll = scroll.saturating_sub(1); }
        }
    }

    fn scroll_selected_into_view(&mut self) {
        let x = self.selected as f32 * 241.0 - self.scroll_x;
        if x < 0.0 {
            self.scroll_x += x;
        } else if x + 228.0 > self.view_w - 210.0 {
            self.scroll_x += x + 228.0 - (self.view_w - 210.0);
        }
        self.scroll_x = self.scroll_x.max(0.0);
    }

    fn hit(&mut self, x: f32, y: f32, w: f32, h: f32, action: Hit) {
        self.hits.push((x, y, w, h, action));
    }

    /* ===================== rendering ===================== */

    fn draw(&mut self) {
        let Some(window) = self.window.clone() else { return };
        let size = window.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));
        let pm = self.render_frame(w, h);

        if let Ok(dir) = std::env::var("CLIPDECK_DUMP_UI") {
            let _ = pm.save_png(std::path::Path::new(&dir).join(format!(
                "ui-{}.png",
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
            )));
        }
        if let Some(surface) = self.surface.as_mut() {
            if let Err(error) = surface.present(&pm) {
                eprintln!("present overlay: {error}");
            }
        }
    }


    fn render_frame(&mut self, w: u32, h: u32) -> Pixmap {
        // Stage a display-only drag preview after initial focus events for QA.
        if std::env::var_os("CLIPDECK_DUMP_UI").is_some()
            && std::env::var("CLIPDECK_DUMP_BOARD_DIALOG").as_deref() == Ok("drag") {
            if let (Some(item), Some(board)) = (self.items.first(), self.pinboards.first()) {
                self.drag = Some(CardDrag { id: item.id.clone(), start: (250.0, 160.0), active: true, board_id: Some(board.id.clone()) });
                self.mouse = Some((100.0, 125.0));
            }
        }
        self.view_w = w as f32;
        let mut pm = Pixmap::new(w, h).expect("pixmap");
        self.hits.clear();
        pm.fill(Color::from_rgba8(16, 18, 28, 255));
        round_rect(&mut pm, 6.0, 6.0, w as f32 - 12.0, h as f32 - 12.0, 18.0, rgba(16, 18, 28, 255));
        stroke_round(&mut pm, 6.0, 6.0, w as f32 - 12.0, h as f32 - 12.0, 18.0, rgba(255, 255, 255, 14), 1.0);
        self.layout(&mut pm, w as f32, h as f32);
        if let Some(path) = rounded_path(6.0, 6.0, w as f32 - 12.0, h as f32 - 12.0, 18.0) {
            let mut mask = tiny_skia::Mask::new(w, h).expect("window mask");
            mask.fill_path(&path, tiny_skia::FillRule::Winding, true, tiny_skia::Transform::identity());
            pm.apply_mask(&mask);
        }
        pm

    }

    fn layout(&mut self, pm: &mut Pixmap, w: f32, h: f32) {
        let f = self.fonts.clone();
        // sidebar
        f.text_l(pm, 30.0, 34.0, 13.0, "History", Face::Regular, rgba(238, 240, 246, 235));
        self.hit(18.0, 20.0, 150.0, 28.0, Hit::SetFilter(0));
        if self.filter == 0 {
            round_rect(pm, 18.0, 18.0, 152.0, 32.0, 10.0, rgba(143, 176, 255, 46));
        }
        f.text_l(pm, 30.0, 66.0, 13.0, "Pinned", Face::Regular, rgba(238, 240, 246, 190));
        self.hit(18.0, 52.0, 150.0, 28.0, Hit::SetFilter(1));
        if self.filter == 1 {
            round_rect(pm, 18.0, 50.0, 152.0, 32.0, 10.0, rgba(143, 176, 255, 46));
        }
        f.text_l(pm, 30.0, 100.0, 10.0, "PINBOARDS", Face::Medium, rgba(238, 240, 246, 120));
        let pinboards = self.pinboards.clone();
        for (n, pb) in pinboards.iter().enumerate() {
            let y = 118.0 + n as f32 * 28.0;
            if self.filter == n + 2 {
                round_rect(pm, 18.0, y - 8.0, 152.0, 30.0, 10.0, rgba(143, 176, 255, 46));
            }
            if self.drag.as_ref().is_some_and(|drag| drag.active && drag.board_id.as_deref() == Some(pb.id.as_str())) {
                round_rect(pm, 18.0, y - 8.0, 152.0, 28.0, 9.0, rgba(143, 176, 255, 90));
                stroke_round(pm, 18.0, y - 8.0, 152.0, 28.0, 9.0, rgba(143, 176, 255, 230), 1.0);
            }
            if let Some(feedback) = self.drop_feedback.as_ref().filter(|feedback| feedback.board_id == pb.id) {
                let strength = feedback.strength(std::time::Instant::now());
                if strength > 0.0 {
                    let expand = 2.0 * (1.0 - strength);
                    round_rect(pm, 18.0, y - 8.0, 152.0, 28.0, 9.0, rgba(105, 220, 157, (100.0 * strength) as u8));
                    stroke_round(pm, 18.0 - expand, y - 8.0 - expand, 152.0 + expand * 2.0, 28.0 + expand * 2.0,
                        9.0 + expand, rgba(105, 220, 157, (230.0 * strength) as u8), 1.5);
                }
            }
            let label = f.layout_lines(86.0, 12.0, &pb.name, Face::Regular, 1).into_iter().next().unwrap_or_default();
            f.text_l(pm, 30.0, y + 7.0, 12.0, &label, Face::Regular, rgba(238, 240, 246, 190));
            self.hit(18.0, y - 8.0, 150.0, 30.0, Hit::SetFilter(n + 2));
            icon_edit(pm, 132.0, y + 7.0, rgba(238, 240, 246, 160));
            self.hit(121.0, y - 4.0, 22.0, 22.0, Hit::RenamePinboard(n));
            icon_x(pm, 156.0, y + 7.0, 3.5, rgba(238, 240, 246, 160));
            self.hit(145.0, y - 4.0, 22.0, 22.0, Hit::DeletePinboard(n));
        }
        f.text_l(pm, 30.0, h - 30.0, 12.0, "+ New", Face::Regular, rgba(238, 240, 246, 140));
        self.hit(18.0, h - 44.0, 150.0, 28.0, Hit::NewPinboard);
        line(pm, 182.0, 16.0, 182.0, h - 12.0, rgba(255, 255, 255, 22));

        if self.settings {
            self.draw_settings(pm, w, h);
            return;
        }

        // topbar
        round_rect(pm, 196.0, 18.0, (w - 316.0 - 196.0).max(120.0), 34.0, 17.0, rgba(255, 255, 255, 16));
        let q = if self.query.is_empty() { "Search history\u{2026}".to_string() } else { self.query.clone() };
        let dim = if self.query.is_empty() { 130 } else { 235 };
        f.text_l(pm, 212.0, 35.0, 13.0, &q, Face::Regular, rgba(238, 240, 246, dim));

        let stacked = self.app.store.items.iter().filter(|i| i.stack_order.is_some()).count();
        if stacked > 0 {
            round_rect(pm, w - 306.0, 18.0, 92.0, 34.0, 17.0, rgba(143, 176, 255, 30));
            f.text_c(pm, w - 260.0, 35.0, 12.0, &format!("Stack x{stacked}"), Face::Medium, rgba(143, 176, 255, 235));
            self.hit(w - 306.0, 18.0, 92.0, 34.0, Hit::StackToggle);
        }
        round_rect(pm, w - 150.0, 18.0, 60.0, 34.0, 17.0, rgba(255, 255, 255, 16));
        f.text_c(pm, w - 120.0, 35.0, 11.0, "Settings", Face::Regular, rgba(238, 240, 246, 225));
        self.hit(w - 150.0, 18.0, 60.0, 34.0, Hit::OpenSettings);
        round_rect(pm, w - 86.0, 18.0, 34.0, 34.0, 17.0, rgba(255, 90, 110, 36));
        icon_x(pm, w - 69.0, 35.0, 4.5, rgba(255, 123, 141, 235));
        self.hit(w - 86.0, 18.0, 34.0, 34.0, Hit::CloseWindow);

        // cards strip
        let card_h = (h - 64.0 - 46.0).clamp(180.0, 252.0);
        let total_w = self.items.len() as f32 * 241.0;
        let visible_w = w - 210.0;
        let max_scroll = (total_w - visible_w).max(0.0);
        if self.scroll_x > max_scroll {
            self.scroll_x = max_scroll;
        }
        let mut x = 196.0 - self.scroll_x;
        let cards = self.items.clone();
        for (i, item) in cards.iter().enumerate() {
            if x > w - 30.0 {
                break;
            }
            if x + 228.0 > 196.0 {
                self.draw_card(pm, x, 64.0, 228.0, card_h, i, item);
            }
            x += 241.0;
        }
        if max_scroll > 0.0 {
            let fade_w = 46.0f32;
            for i in 0..fade_w as u32 {
                let t = i as f32 / fade_w;
                let a = (255.0 * t * t) as u8;
                line(pm, w - fade_w + i as f32, 60.0, w - fade_w + i as f32, 60.0 + card_h, rgba(10, 11, 18, a));
            }
        }
        if self.items.is_empty() {
            f.text_c(pm, (w + 196.0) / 2.0, h / 2.0, 15.0, "Clipboard history is empty", Face::Medium, rgba(238, 240, 246, 200));
            f.text_c(pm, (w + 196.0) / 2.0, h / 2.0 + 26.0, 12.0, "Copy anything \u{2014} it shows up here instantly.", Face::Regular, rgba(238, 240, 246, 130));
        }
        self.draw_keyboard_hints(pm, w, h);

        if self.stack_open {
            self.draw_stack(pm, w);
        }
        self.draw_drop_confirmation(pm, w, h);
        self.draw_drag_preview(pm, w, h);
        self.draw_dialog(pm, w, h);
    }

    fn draw_keyboard_hints(&self, pm: &mut Pixmap, w: f32, h: f32) {
        let f = &self.fonts;
        round_rect(pm, 196.0, h - 40.0, w - 216.0, 30.0, 8.0, rgba(28, 31, 45, 255));
        if self.drop_feedback.as_ref().is_some_and(|feedback| !feedback.expired(std::time::Instant::now())
            && self.pinboards.iter().any(|board| board.id == feedback.board_id)) { return; }
        if self.drag.as_ref().is_some_and(|drag| drag.active) {
            f.text_c(pm, (w + 196.0) / 2.0, h - 25.0, 12.0,
                "Drop onto a sidebar board to move · Release elsewhere to cancel", Face::Medium, rgba(238, 240, 246, 245));
            return;
        }
        let hints = KEYBOARD_HINTS;
        let total: f32 = hints.iter().map(|(key, label)| f.width(12.0, key, Face::Medium)
            + f.width(12.0, label, Face::Regular) + 32.0).sum();
        let mut x = 196.0 + ((w - 216.0 - total) / 2.0).max(8.0);
        for (key, label) in hints {
            let key_width = f.width(12.0, key, Face::Medium) + 12.0;
            round_rect(pm, x, h - 35.0, key_width, 20.0, 4.0, rgba(255, 255, 255, 25));
            f.text_c(pm, x + key_width / 2.0, h - 25.0, 12.0, key, Face::Medium, rgba(250, 251, 255, 255));
            f.text_l(pm, x + key_width + 6.0, h - 25.0, 12.0, label, Face::Regular, rgba(224, 228, 240, 245));
            x += key_width + 6.0 + f.width(12.0, label, Face::Regular) + 14.0;
        }
    }

    fn draw_drop_confirmation(&mut self, pm: &mut Pixmap, w: f32, h: f32) {
        let Some(feedback) = &self.drop_feedback else { return };
        let opacity = feedback.opacity(std::time::Instant::now());
        if opacity == 0.0 { return; }
        let Some(board) = self.pinboards.iter().find(|board| board.id == feedback.board_id) else { return };
        let text = self.fonts.layout_lines(350.0, 11.0, &format!("Moved to {}", board.name), Face::Medium, 1)
            .into_iter().next().unwrap_or_default();
        let width = (self.fonts.width(11.0, &text, Face::Medium) + 28.0).ceil() as u32;
        let Some(mut toast) = Pixmap::new(width, 26) else { return };
        round_rect(&mut toast, 0.0, 0.0, width as f32, 26.0, 8.0, rgba(30, 70, 50, 255));
        self.fonts.text_c(&mut toast, width as f32 / 2.0, 13.0, 11.0, &text, Face::Medium, rgba(210, 250, 225, 255));
        let paint = tiny_skia::PixmapPaint { opacity, ..Default::default() };
        pm.draw_pixmap(((w + 196.0 - width as f32) / 2.0) as i32, (h - 38.0) as i32,
            toast.as_ref(), &paint, tiny_skia::Transform::identity(), None);
    }

    fn draw_dialog(&mut self, pm: &mut Pixmap, w: f32, h: f32) {
        let Some(dialog) = self.dialog.clone() else { return };
        let f = self.fonts.clone();
        self.hits.clear(); // Dialogs own input; no card or sidebar click-through.
        round_rect(pm, 0.0, 0.0, w, h, 0.0, rgba(0, 0, 0, 165));
        self.hit(0.0, 0.0, w, h, Hit::CancelDialog);
        let rows = board_picker_rows(h).min(self.pinboards.len() + 1);
        let panel_h = match dialog {
            Dialog::Rename { .. } => 170.0,
            Dialog::Move { .. } => 88.0 + rows as f32 * 28.0,
            Dialog::ConfirmDelete { target: DeleteTarget::Clip { board_id: Some(_), .. } } => 218.0,
            Dialog::ConfirmDelete { .. } => 190.0,
        };
        let x = (w - 360.0) / 2.0;
        let y = (h - panel_h) / 2.0;
        round_rect(pm, x, y, 360.0, panel_h, 14.0, rgba(28, 31, 45, 255));
        self.hit(x, y, 360.0, panel_h, Hit::DialogBackground);
        match dialog {
            Dialog::Rename { draft, select_all, .. } => {
                f.text_l(pm, x + 20.0, y + 25.0, 15.0, "Rename board", Face::Medium, rgba(238, 240, 246, 240));
                round_rect(pm, x + 18.0, y + 47.0, 324.0, 36.0, 8.0, rgba(255, 255, 255, 18));
                if select_all && !draft.is_empty() {
                    round_rect(pm, x + 25.0, y + 54.0, f.width(14.0, &draft, Face::Regular).min(306.0), 22.0, 3.0, rgba(109, 149, 255, 100));
                }
                let text = f.layout_lines(304.0, 14.0, &draft, Face::Regular, 1).into_iter().next().unwrap_or_default();
                f.text_l(pm, x + 28.0, y + 65.0, 14.0, &text, Face::Regular, rgba(238, 240, 246, 245));
                let hint = if draft.trim().is_empty() { "Enter a board name" } else { "Type a name · Enter to save" };
                f.text_l(pm, x + 20.0, y + 100.0, 11.0, hint, Face::Regular, rgba(238, 240, 246, 150));
                round_rect(pm, x + 238.0, y + 124.0, 102.0, 28.0, 8.0, rgba(109, 149, 255, 235));
                f.text_c(pm, x + 289.0, y + 138.0, 12.0, "Save", Face::Medium, rgba(255, 255, 255, 255));
                self.hit(x + 238.0, y + 124.0, 102.0, 28.0, Hit::SaveBoardName);
            }
            Dialog::ConfirmDelete { target } => {
                let (title, label, detail, from_board, delete_history) = match target {
                    DeleteTarget::Clip { label, board_id: Some(_), delete_history, .. } => (
                        "Remove clip from board?", label,
                        if delete_history { "This also deletes the clip from History." } else { "The clip will stay in History." },
                        true, delete_history,
                    ),
                    DeleteTarget::Clip { label, .. } => ("Delete clip?", label, "This removes the clip from history.", false, true),
                    DeleteTarget::Board { label, .. } => ("Delete board?", label, "Your clips will stay in History.", false, true),
                };
                f.text_l(pm, x + 20.0, y + 25.0, 15.0, title, Face::Medium, rgba(238, 240, 246, 240));
                let label: String = label.chars().take(160).collect();
                f.wrap_lines(pm, x + 20.0, y + 50.0, 320.0, 12.0, &label, Face::Regular, 2);
                f.text_l(pm, x + 20.0, y + 108.0, 11.0, detail, Face::Regular, rgba(238, 240, 246, 180));
                if from_board {
                    let color = if delete_history { rgba(190, 65, 85, 255) } else { rgba(255, 255, 255, 24) };
                    round_rect(pm, x + 20.0, y + 126.0, 18.0, 18.0, 4.0, color);
                    stroke_round(pm, x + 20.0, y + 126.0, 18.0, 18.0, 4.0, rgba(238, 240, 246, 170), 1.0);
                    if delete_history {
                        line(pm, x + 24.0, y + 135.0, x + 28.0, y + 139.0, rgba(255, 255, 255, 255));
                        line(pm, x + 28.0, y + 139.0, x + 34.0, y + 131.0, rgba(255, 255, 255, 255));
                    }
                    f.text_l(pm, x + 48.0, y + 135.0, 12.0, "Also delete from History", Face::Regular, rgba(238, 240, 246, 225));
                    self.hit(x + 18.0, y + 122.0, 324.0, 26.0, Hit::ToggleDeleteHistory);
                }
                let destructive = !from_board || delete_history;
                let color = if destructive { rgba(190, 65, 85, 255) } else { rgba(109, 149, 255, 235) };
                round_rect(pm, x + 238.0, y + panel_h - 46.0, 102.0, 28.0, 8.0, color);
                f.text_c(pm, x + 289.0, y + panel_h - 32.0, 12.0, if destructive { "Delete" } else { "Remove" }, Face::Medium, rgba(255, 255, 255, 255));
                self.hit(x + 238.0, y + panel_h - 46.0, 102.0, 28.0, Hit::ConfirmDelete);
            }
            Dialog::Move { item_id, scroll } => {
                f.text_l(pm, x + 20.0, y + 25.0, 15.0, "Move clip to board", Face::Medium, rgba(238, 240, 246, 240));
                let current = self.app.store.find(&item_id).and_then(|item| item.pinboard_id.clone());
                let scroll = scroll.min((self.pinboards.len() + 1).saturating_sub(rows));
                for (row, n) in (scroll..self.pinboards.len() + 1).take(rows).enumerate() {
                    let (name, selected) = if n == 0 {
                        ("No board (History)".to_owned(), current.is_none())
                    } else {
                        let board = &self.pinboards[n - 1];
                        (board.name.clone(), current.as_deref() == Some(board.id.as_str()))
                    };
                    let cy = y + 48.0 + row as f32 * 28.0;
                    if selected { round_rect(pm, x + 14.0, cy - 12.0, 332.0, 25.0, 6.0, rgba(143, 176, 255, 40)); }
                    let label = f.layout_lines(280.0, 12.0, &name, Face::Regular, 1).into_iter().next().unwrap_or_default();
                    f.text_l(pm, x + 24.0, cy, 12.0, &label, Face::Regular, rgba(238, 240, 246, 230));
                    if selected { f.text_l(pm, x + 320.0, cy, 12.0, "•", Face::Medium, rgba(143, 176, 255, 235)); }
                    self.hit(x + 14.0, cy - 12.0, 332.0, 25.0, Hit::ChoosePinboard(n));
                }
                if self.pinboards.len() + 1 > rows {
                    f.text_l(pm, x + 20.0, y + panel_h - 21.0, 10.0, "Scroll for more boards", Face::Regular, rgba(238, 240, 246, 150));
                }
            }
        }
        let cancel_x = match self.dialog { Some(Dialog::Rename { .. } | Dialog::ConfirmDelete { .. }) => x + 126.0, _ => x + 238.0 };
        round_rect(pm, cancel_x, y + panel_h - 46.0, 102.0, 28.0, 8.0, rgba(255, 255, 255, 20));
        f.text_c(pm, cancel_x + 51.0, y + panel_h - 32.0, 12.0, "Cancel", Face::Regular, rgba(238, 240, 246, 220));
        self.hit(cancel_x, y + panel_h - 46.0, 102.0, 28.0, Hit::CancelDialog);
    }

    fn draw_drag_preview(&mut self, pm: &mut Pixmap, w: f32, h: f32) {
        let Some(drag) = self.drag.clone().filter(|drag| drag.active) else { return };
        let Some((mx, my)) = self.mouse else { return };
        let Some(item) = self.app.store.find(&drag.id).cloned() else { return };
        let mut card = Pixmap::new(228, 252).expect("drag card");
        card.fill(Color::from_rgba8(16, 18, 28, 255));
        let first_hit = self.hits.len();
        self.draw_card_contents(&mut card, 0.0, 0.0, 228.0, 252.0, self.selected, &item);
        self.hits.truncate(first_hit); // The floating card must never intercept a drop.
        let Some(mut preview) = scale_box(&card, 171, 189) else { return };
        let path = rounded_path(0.0, 0.0, 171.0, 189.0, 11.0).expect("drag outline");
        let mut mask = tiny_skia::Mask::new(171, 189).expect("drag mask");
        mask.fill_path(&path, tiny_skia::FillRule::Winding, true, tiny_skia::Transform::identity());
        preview.apply_mask(&mask);
        let x = (mx + 18.0).clamp(8.0, (w - 187.0).max(8.0));
        let y = (my - 20.0).clamp(8.0, (h - 217.0).max(8.0));
        round_rect(pm, x + 4.0, y + 8.0, 175.0, 193.0, 13.0, rgba(0, 0, 0, 70));
        round_rect(pm, x + 2.0, y + 4.0, 173.0, 191.0, 12.0, rgba(0, 0, 0, 100));
        let paint = tiny_skia::PixmapPaint { opacity: 0.92, ..Default::default() };
        pm.draw_pixmap(x as i32, y as i32, preview.as_ref(), &paint, tiny_skia::Transform::identity(), None);
        stroke_round(pm, x, y, 171.0, 189.0, 11.0, rgba(143, 176, 255, 235), 1.5);
    }

    fn draw_card(&mut self, pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, idx: usize, item: &ClipItem) {
        // A local raster bounds every primitive, including text's direct pixel blits.
        let Some(mut card) = Pixmap::new(w.ceil() as u32, h.ceil() as u32) else { return };
        card.fill(Color::from_rgba8(16, 18, 28, 255));
        let first_hit = self.hits.len();
        self.draw_card_contents(&mut card, 0.0, 0.0, w, h, idx, item);
        if self.drag.as_ref().is_some_and(|drag| drag.active && drag.id == item.id) {
            round_rect(&mut card, 0.0, 0.0, w, h, 15.0, rgba(0, 0, 0, 100));
        }
        let left = x.max(196.0);
        let right = (x + w).min(pm.width() as f32 - 30.0);
        if right <= left {
            self.hits.truncate(first_hit);
            return;
        }
        for (hx, hy, hw, _, _) in &mut self.hits[first_hit..] {
            let end = (*hx + x + *hw).min(right);
            *hx = (*hx + x).max(left);
            *hy += y;
            *hw = (end - *hx).max(0.0);
        }
        let mut mask = tiny_skia::Mask::new(pm.width(), pm.height()).expect("card mask");
        let rect = tiny_skia::Rect::from_xywh(left, y, right - left, h).expect("card clip");
        let path = tiny_skia::PathBuilder::from_rect(rect);
        mask.fill_path(&path, tiny_skia::FillRule::Winding, false, tiny_skia::Transform::identity());
        pm.draw_pixmap(x as i32, y as i32, card.as_ref(), &tiny_skia::PixmapPaint::default(), tiny_skia::Transform::identity(), Some(&mask));
    }

    fn draw_card_contents(&mut self, pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, idx: usize, item: &ClipItem) {
        let f = self.fonts.clone();
        let selected = idx == self.selected;
        round_rect(pm, x, y, w, h, 15.0, rgba(255, 255, 255, 14));
        if selected {
            stroke_round(pm, x, y, w, h, 15.0, rgba(143, 176, 255, 230), 1.5);
        }
        self.hit(x, y, w, h, Hit::Select(idx));
        f.text_l(pm, x + 14.0, y + 13.0, 10.0, "Move to…", Face::Regular, rgba(143, 176, 255, 220));
        self.hit(x + 8.0, y + 2.0, 76.0, 22.0, Hit::MoveToBoard(idx));
        icon_pin(pm, x + w - 61.0, y + 13.0, item.pinned);
        self.hit(x + w - 72.0, y + 2.0, 22.0, 22.0, Hit::Pin(idx));
        icon_edit(pm, x + w - 37.0, y + 13.0, rgba(238, 240, 246, 190));
        self.hit(x + w - 48.0, y + 2.0, 22.0, 22.0, Hit::Edit(idx));
        icon_x(pm, x + w - 15.0, y + 13.0, 4.0, rgba(238, 240, 246, 190));
        self.hit(x + w - 26.0, y + 2.0, 22.0, 22.0, Hit::Delete(idx));

        let py = y + 30.0;
        let ph = h - 66.0;
        if let Some((id, draft)) = &self.editing {
            if id == &item.id {
                round_rect(pm, x + 8.0, py, w - 16.0, ph, 10.0, rgba(0, 0, 0, 60));
                stroke_round(pm, x + 8.0, py, w - 16.0, ph, 10.0, rgba(143, 176, 255, 230), 1.2);
                f.wrap_lines(pm, x + 16.0, py + 8.0, w - 32.0, 11.0, draft, Face::Mono, fit_lines(ph - 42.0, 11.0));
                round_rect(pm, x + 12.0, y + h - 30.0, 84.0, 22.0, 8.0, rgba(109, 149, 255, 235));
                f.text_c(pm, x + 54.0, y + h - 19.0, 11.0, "Save", Face::Medium, rgba(255, 255, 255, 255));
                self.hit(x + 12.0, y + h - 30.0, 84.0, 22.0, Hit::SaveEdit);
                round_rect(pm, x + 104.0, y + h - 30.0, 84.0, 22.0, 8.0, rgba(255, 255, 255, 20));
                f.text_c(pm, x + 146.0, y + h - 19.0, 11.0, "Cancel", Face::Medium, rgba(238, 240, 246, 220));
                self.hit(x + 104.0, y + h - 30.0, 84.0, 22.0, Hit::CancelEdit);
                return;
            }
        }

        match item.item_type {
            clipdeck_core::ItemType::Color => {
                round_rect(pm, x + 8.0, py, w - 16.0, ph, 10.0, parse_color(&item.content));
            }
            clipdeck_core::ItemType::Image => {
                if let Some(name) = &item.image_file {
                    if let Some(image) = self.thumbnails.get(
                        &self.app.store.image_dir, name, (w - 16.0) as u32, ph as u32, self.app.generation,
                    ) {
                        let dx = x + 8.0 + ((w - 16.0) - image.width() as f32) / 2.0;
                        let dy = py + (ph - image.height() as f32) / 2.0;
                        pm.draw_pixmap(dx as i32, dy as i32, image.as_ref(), &tiny_skia::PixmapPaint::default(), tiny_skia::Transform::identity(), None);
                    }
                }
            }
            clipdeck_core::ItemType::Link => {
                round_rect(pm, x + 16.0, py + 14.0, 42.0, 42.0, 11.0, rgba(109, 149, 255, 235));
                let letter: String = item.preview.chars().take(1).collect();
                f.text_c(pm, x + 37.0, py + 35.0, 17.0, &letter, Face::Medium, rgba(255, 255, 255, 255));
                f.wrap_lines(pm, x + 16.0, py + 72.0, w - 32.0, 13.0, &item.title, Face::Regular, fit_lines(ph - 96.0, 13.0).min(3));
                if item.preview != item.title && ph >= 146.0 {
                    f.text_l(pm, x + 16.0, py + 134.0, 11.0, &item.preview, Face::Regular, rgba(238, 240, 246, 140));
                }
            }
            clipdeck_core::ItemType::File => {
                let rows = (((ph - 12.0) / 37.0).floor() as usize).clamp(1, 4);
                let mut ly = py + 12.0;
                for path in item.content.lines().take(rows) {
                    round_rect(pm, x + 12.0, ly, w - 24.0, 30.0, 8.0, rgba(255, 255, 255, 14));
                    let name = path.rsplit('/').next().unwrap_or(path);
                    f.text_l(pm, x + 22.0, ly + 15.0, 11.0, name, Face::Regular, rgba(238, 240, 246, 210));
                    ly += 37.0;
                }
            }
            clipdeck_core::ItemType::Code => {
                round_rect(pm, x + 8.0, py, w - 16.0, ph, 10.0, rgba(0, 0, 0, 60));
                f.wrap_lines(pm, x + 16.0, py + 12.0, w - 32.0, 12.0, &item.content, Face::Mono, fit_lines(ph - 24.0, 12.0));
            }
            _ => {
                f.wrap_lines(pm, x + 14.0, py + 8.0, w - 28.0, 13.0, &item.content, Face::Regular, fit_lines(ph - 16.0, 13.0));
            }
        }

        let badge = match item.item_type {
            clipdeck_core::ItemType::Text => "Text",
            clipdeck_core::ItemType::Link => "Link",
            clipdeck_core::ItemType::Code => "Code",
            clipdeck_core::ItemType::Color => "Color",
            clipdeck_core::ItemType::Image => "Image",
            clipdeck_core::ItemType::File => "Files",
            clipdeck_core::ItemType::Html => "Rich text",
        };
        let badge_w = f.width(10.0, badge, Face::Medium) + 16.0;
        round_rect(pm, x + 12.0, y + h - 28.0, badge_w, 18.0, 7.0, rgba(143, 176, 255, 38));
        f.text_c(pm, x + 12.0 + badge_w / 2.0, y + h - 19.0, 10.0, badge, Face::Medium, rgba(143, 176, 255, 235));
        let age = time_ago(item.created_at);
        f.text_l(pm, x + 18.0 + badge_w, y + h - 19.0, 10.0, &age, Face::Regular, rgba(238, 240, 246, 130));
        if let Some(n) = item.stack_order {
            f.text_l(pm, x + 62.0 + badge_w, y + h - 19.0, 10.0, &format!("#{n}"), Face::Medium, rgba(143, 176, 255, 235));
        }
    }

    fn draw_stack(&mut self, pm: &mut Pixmap, w: f32) {
        let f = self.fonts.clone();
        let queued = self.app.store.stack_items();
        let x = w - 360.0;
        let y = 60.0;
        round_rect(pm, x, y, 330.0, 240.0, 14.0, rgba(22, 25, 38, 250));
        f.text_l(pm, x + 14.0, y + 14.0, 12.0, &format!("Paste stack ({})", queued.len()), Face::Medium, rgba(238, 240, 246, 235));
        f.text_c(pm, x + 256.0, y + 14.0, 11.0, "clear", Face::Regular, rgba(238, 240, 246, 170));
        self.hit(x + 234.0, y + 4.0, 44.0, 22.0, Hit::StackClear);
        for (i, item) in queued.iter().enumerate().take(5) {
            let ry = y + 40.0 + i as f32 * 30.0;
            f.text_c(pm, x + 24.0, ry + 12.0, 11.0, &format!("{}", i + 1), Face::Medium, rgba(143, 176, 255, 235));
            f.text_l(pm, x + 44.0, ry + 12.0, 11.5, &item.title, Face::Regular, rgba(238, 240, 246, 210));
            f.text_c(pm, x + 284.0, ry + 12.0, 11.0, "x", Face::Regular, rgba(238, 240, 246, 170));
            self.hit(x + 272.0, ry, 24.0, 24.0, Hit::StackRemove(i));
        }
        round_rect(pm, x + 12.0, y + 200.0, 306.0, 30.0, 9.0, rgba(109, 149, 255, 235));
        f.text_c(pm, x + 165.0, y + 215.0, 12.0, "Paste next", Face::Medium, rgba(255, 255, 255, 255));
        self.hit(x + 12.0, y + 200.0, 306.0, 30.0, Hit::StackPasteNext);
    }

    fn draw_shortcut_setup(&mut self, pm: &mut Pixmap, x: f32, y: f32) {
        let f = self.fonts.clone();
        round_rect(pm, x, y, 440.0, 300.0, 12.0, rgba(28, 31, 45, 255));
        f.text_l(pm, x + 18.0, y + 22.0, 14.0, "Global shortcut", Face::Medium, rgba(245, 247, 252, 255));
        let description = if self.wayland_session {
            "On GNOME Wayland, create a system shortcut to show or hide ClipDeck."
        } else {
            "Record a shortcut on the left, or create a GNOME system shortcut below."
        };
        f.wrap_lines(pm, x + 18.0, y + 40.0, 404.0, 12.0, description, Face::Regular, 2);
        f.wrap_lines(pm, x + 18.0, y + 84.0, 404.0, 11.0,
            "Keyboard Settings > View and Customize Shortcuts > Custom Shortcuts > +", Face::Regular, 2);
        f.text_l(pm, x + 18.0, y + 132.0, 11.0, "Name: ClipDeck · Shortcut: Ctrl+Shift+V (or unused keys)", Face::Regular, rgba(238, 240, 246, 230));
        f.text_l(pm, x + 18.0, y + 155.0, 11.0, "Command:", Face::Medium, rgba(238, 240, 246, 200));
        round_rect(pm, x + 18.0, y + 168.0, 404.0, 44.0, 6.0, rgba(0, 0, 0, 70));
        f.wrap_lines(pm, x + 26.0, y + 174.0, 388.0, 10.5, &shortcut_command(), Face::Mono, 2);
        round_rect(pm, x + 18.0, y + 226.0, 118.0, 30.0, 7.0, rgba(255, 255, 255, 24));
        f.text_c(pm, x + 77.0, y + 241.0, 11.0, "Copy Command", Face::Medium, rgba(245, 247, 252, 255));
        self.hit(x + 18.0, y + 226.0, 118.0, 30.0, Hit::CopyShortcutCommand);
        round_rect(pm, x + 146.0, y + 226.0, 276.0, 30.0, 7.0, rgba(109, 149, 255, 235));
        f.text_c(pm, x + 284.0, y + 241.0, 12.0, "Open Keyboard Settings", Face::Medium, rgba(255, 255, 255, 255));
        self.hit(x + 146.0, y + 226.0, 276.0, 30.0, Hit::OpenKeyboardSettings);
        let note = self.settings_notice.as_deref().unwrap_or("Keep ClipDeck running for this shortcut.");
        f.wrap_lines(pm, x + 18.0, y + 268.0, 404.0, 10.5, note, Face::Regular, 2);
    }

    fn setting_unavailable_reason(&self, key: &str) -> Option<&'static str> {
        match key {
            "transient" | "share" | "link_previews" => Some("Unsupported"),
            "confidential" if self.wayland_session => Some("No source app"),
            _ => None,
        }
    }

    fn draw_settings(&mut self, pm: &mut Pixmap, w: f32, h: f32) {
        let f = self.fonts.clone();
        let s = self.app.settings().clone();
        let left = 210.0f32;
        f.text_l(pm, left, 30.0, 15.0, "ClipDeck Settings", Face::Medium, rgba(238, 240, 246, 245));

        let rows: [(&str, bool, Hit); 11] = [
            ("Watch clipboard", s.watch_enabled, Hit::Toggle("watch")),
            ("Play sound on copy", s.sound_copy, Hit::Toggle("sound_copy")),
            ("Play sound on paste", s.sound_paste, Hit::Toggle("sound_paste")),
            ("Ignore transient content", s.ignore_transient, Hit::Toggle("transient")),
            ("Confidential-app filter", s.ignore_confidential, Hit::Toggle("confidential")),
            ("Generate link previews", s.generate_link_previews, Hit::Toggle("link_previews")),
            ("Always paste as plain text", s.paste_as_plain_text, Hit::Toggle("plain")),
            ("Start ClipDeck at login", s.launch_at_login, Hit::Toggle("login")),
            ("Hide overlay from screen sharing", s.show_in_screen_share, Hit::Toggle("share")),
            ("Paste stack: LIFO (off = FIFO)", s.stack_direction == "lifo", Hit::Toggle("stack_dir")),
            ("Global shortcut", false, Hit::ShortcutCapture),
        ];
        let row_h = 24.0f32;
        let top = 52.0f32;
        let panel_x = (w - 470.0).max(left + 330.0);
        let controls_right = panel_x - 30.0;
        for (n, (label, on, hit)) in rows.iter().enumerate() {
            let y = top + n as f32 * row_h;
            if *hit == Hit::ShortcutCapture && self.wayland_session { continue; }
            let unavailable = match hit {
                Hit::Toggle(key) => self.setting_unavailable_reason(key),
                _ => None,
            };
            f.text_l(pm, left, y + 12.0, 12.0, label, Face::Regular,
                rgba(238, 240, 246, if unavailable.is_some() { 160 } else { 220 }));
            if let Some(reason) = unavailable {
                let status_x = controls_right - 108.0;
                round_rect(pm, status_x, y + 1.0, 108.0, 22.0, 7.0, rgba(255, 255, 255, 12));
                f.text_c(pm, status_x + 54.0, y + 12.0, 11.0, reason, Face::Regular, rgba(238, 240, 246, 180));
                continue;
            }
            match hit {
                Hit::ShortcutCapture => {
                    let shown = if self.capture { "Press keys\u{2026} (Esc cancels)".to_string() } else { pretty_accel(&s.shortcut) };
                    let bx = (controls_right - 180.0).max(left + 160.0);
                    let bw = controls_right - bx;
                    round_rect(pm, bx, y + 1.0, bw, 22.0, 7.0, if self.capture { rgba(143, 176, 255, 46) } else { rgba(255, 255, 255, 16) });
                    f.text_c(pm, bx + bw / 2.0, y + 12.0, 11.0, &shown, Face::Medium, rgba(238, 240, 246, 235));
                }
                _ => {
                    let tx = controls_right - 50.0;
                    let bg = if *on { rgba(109, 149, 255, 235) } else { rgba(255, 255, 255, 26) };
                    round_rect(pm, tx, y + 1.0, 50.0, 22.0, 11.0, bg);
                    round_rect(pm, if *on { tx + 30.0 } else { tx + 3.0 }, y + 4.0, 16.0, 16.0, 8.0, rgba(255, 255, 255, 245));
                }
            }
            if *hit != Hit::ShortcutCapture || !self.wayland_session {
                self.hit(left, y, controls_right - left, row_h, *hit);
            }
        }
        if self.wayland_session {
            f.wrap_lines(pm, left, top + 10.0 * row_h, controls_right - left, 11.0,
                "Clipboard source is unknown on Wayland. App exclusions and confidential-app filtering cannot protect these captures.",
                Face::Regular, 2);
        }
        self.draw_shortcut_setup(pm, panel_x, top);
        let by = top + rows.len() as f32 * row_h + 10.0;
        if by + 28.0 <= h - 6.0 {
            round_rect(pm, left, by, 104.0, 28.0, 8.0, rgba(255, 255, 255, 16));
            f.text_c(pm, left + 52.0, by + 14.0, 12.0, "Export", Face::Medium, rgba(238, 240, 246, 235));
            self.hit(left, by, 104.0, 28.0, Hit::Export);
            round_rect(pm, left + 116.0, by, 104.0, 28.0, 8.0, rgba(255, 255, 255, 16));
            f.text_c(pm, left + 168.0, by + 14.0, 12.0, "Import", Face::Medium, rgba(238, 240, 246, 235));
            self.hit(left + 116.0, by, 104.0, 28.0, Hit::Import);
        }
    }
}

fn keyboard_settings_command() -> std::process::Command {
    let mut command = std::process::Command::new("gnome-control-center");
    command.arg("keyboard");
    command
}

fn shortcut_command() -> String {
    std::env::current_exe().map(|path| {
        let path = path.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{path}\" toggle")
    }).unwrap_or_else(|_| "clipdeck toggle".into())
}

/* ===================== input mapping ===================== */

fn accel_from_key_event(ev: &KeyEvent, mods: ModifiersState) -> Option<String> {
    if matches!(ev.logical_key, Key::Named(NamedKey::Escape)) {
        return None;
    }
    let code = key_code(&ev.logical_key)?;
    let kp = KeyPress {
        ctrl: mods.contains(ModifiersState::CONTROL),
        meta: mods.contains(ModifiersState::SUPER),
        alt: mods.contains(ModifiersState::ALT),
        shift: mods.contains(ModifiersState::SHIFT),
        code,
    };
    if !kp.ctrl && !kp.meta && !kp.alt && !kp.shift {
        return None;
    }
    record_accel(&kp)
}

/// Keyboard routing (pure): the single place that decides what a key means
/// in the default overlay mode. Search typing wins over single-letter
/// shortcuts; navigation keys never require modifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
enum KeyAction {
    Ignore,
    SelectAll,
    QueryPush(String),
    QueryPop,
    Next,
    Prev,
    Activate { plain: bool },
    DeleteSelected,
    CopySelected,
    StackSelected,
    EditSelected,
    Close,
}

fn board_picker_rows(height: f32) -> usize {
    ((height - 140.0) / 28.0).floor().max(1.0) as usize
}

fn route_key(key: &Key, ctrl: bool, shift: bool) -> KeyAction {
    match key {
        Key::Named(NamedKey::Escape) => KeyAction::Close,
        Key::Named(NamedKey::ArrowRight) | Key::Named(NamedKey::Tab) if !shift => KeyAction::Next,
        Key::Named(NamedKey::ArrowLeft) | Key::Named(NamedKey::Tab) if shift => KeyAction::Prev,
        Key::Named(NamedKey::ArrowLeft) => KeyAction::Prev,
        Key::Named(NamedKey::ArrowRight) => KeyAction::Next,
        Key::Named(NamedKey::Enter) => KeyAction::Activate { plain: shift },
        Key::Named(NamedKey::Delete) => KeyAction::DeleteSelected,
        Key::Named(NamedKey::Backspace) => KeyAction::QueryPop,
        Key::Character(c) if ctrl => match c.as_str() {
            "a" => KeyAction::SelectAll,
            "c" => KeyAction::CopySelected,
            "s" => KeyAction::StackSelected,
            "e" => KeyAction::EditSelected,
            _ => KeyAction::Ignore,
        },
        Key::Character(c) => KeyAction::QueryPush(c.to_string()),
        _ => KeyAction::Ignore,
    }
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
                    ' ' => "Space".into(), ',' => "Comma".into(), '.' => "Period".into(),
                    '/' => "Slash".into(), '\\' => "Backslash".into(), '-' => "Minus".into(),
                    '=' => "Equal".into(), ';' => "Semicolon".into(), '\'' => "Quote".into(),
                    '`' => "Backquote".into(), '[' => "BracketLeft".into(), ']' => "BracketRight".into(),
                    _ => return None,
                }
            }
        }
        Key::Named(n) => match n {
            NamedKey::Enter => "Enter".into(), NamedKey::Tab => "Tab".into(),
            NamedKey::Backspace => "Backspace".into(), NamedKey::Delete => "Delete".into(),
            NamedKey::ArrowUp => "ArrowUp".into(), NamedKey::ArrowDown => "ArrowDown".into(),
            NamedKey::ArrowLeft => "ArrowLeft".into(), NamedKey::ArrowRight => "ArrowRight".into(),
            NamedKey::Space => "Space".into(),
            NamedKey::F1 => "F1".into(), NamedKey::F2 => "F2".into(), NamedKey::F3 => "F3".into(),
            NamedKey::F4 => "F4".into(), NamedKey::F5 => "F5".into(), NamedKey::F6 => "F6".into(),
            NamedKey::F7 => "F7".into(), NamedKey::F8 => "F8".into(), NamedKey::F9 => "F9".into(),
            NamedKey::F10 => "F10".into(), NamedKey::F11 => "F11".into(), NamedKey::F12 => "F12".into(),
            _ => return None,
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::Key as WinitKey;

    #[test]
    fn pause_dispatch_updates_clipboard_worker() {
        let (mut ui, worker_rx) = fixture_ui_with_worker();
        ui.apply(AppEvent::ToggleWatch);
        assert!(!ui.app.settings().watch_enabled);
        assert!(matches!(worker_rx.try_recv(), Ok(WorkerCmd::SetWatching(false))));
        ui.apply(AppEvent::ToggleWatch);
        assert!(matches!(worker_rx.try_recv(), Ok(WorkerCmd::SetWatching(true))));
    }

    #[test]
    fn keyboard_hints_have_glyphs_in_bundled_fonts() {
        let fonts = Fonts::bundled();
        for (key, label) in KEYBOARD_HINTS {
            for ch in key.chars() {
                assert_ne!(fonts.medium.lookup_glyph_index(ch), 0, "unsupported key glyph {ch}");
            }
            for ch in label.chars() {
                assert_ne!(fonts.regular.lookup_glyph_index(ch), 0, "unsupported label glyph {ch}");
            }
        }
    }

    fn fixture_ui() -> Ui {
        fixture_ui_with_worker().0
    }

    fn fixture_ui_with_worker() -> (Ui, std::sync::mpsc::Receiver<WorkerCmd>) {
        let root = std::env::temp_dir().join(format!("clipdeck-ui-{}", clipdeck_core::store::uuid_v4ish()));
        let (worker_tx, worker_rx) = std::sync::mpsc::channel();
        let (accel_tx, _) = std::sync::mpsc::channel();
        (Ui {
            app: AppState::open(&root), worker_tx, accel_tx,
            tray_view: Arc::new(RwLock::new(TrayView::default())),
            fonts: Fonts::bundled(), thumbnails: ThumbnailCache::default(), window: None, surface: None,
            query: String::new(), selected: 0, filter: 0, editing: None, dialog: None, drag: None, drop_feedback: None,
            settings: false, settings_notice: None, capture: false, wayland_session: true, stack_open: false, scroll_x: 0.0,
            view_w: W, visible: false, mods: ModifiersState::empty(), mouse: None, last_card_click: None,
            hits: vec![], items: vec![], pinboards: vec![],
        }, worker_rx)
    }

    fn fixture_item(kind: clipdeck_core::ItemType, content: &str) -> ClipItem {
        ClipItem {
            id: "qa-item".into(), item_type: kind, content: content.into(),
            title: content.into(), preview: content.into(), source_app: None,
            created_at: 0, size_bytes: content.len() as u64, pinned: false,
            pinboard_id: None, stack_order: None, image_file: None,
        }
    }

    fn card_frame(ui: &mut Ui, item: &ClipItem) -> Pixmap {
        let mut pm = Pixmap::new(W as u32, H as u32).unwrap();
        pm.fill(Color::from_rgba8(16, 18, 28, 255));
        ui.draw_card(&mut pm, 196.0, 64.0, 228.0, 252.0, 0, item);
        pm
    }

    fn click_hit(ui: &mut Ui, hit: Hit) {
        let (x, y, w, h, _) = *ui.hits.iter().rev().find(|(_, _, _, _, action)| *action == hit)
            .expect("requested control is missing");
        ui.mouse = Some((x + w / 2.0, y + h / 2.0));
        ui.click();
    }

    #[test]
    fn toolbar_pin_is_removed_while_card_pin_remains() {
        let mut ui = fixture_ui();
        let item = fixture_item(clipdeck_core::ItemType::Text, "Card pin stays");
        ui.app.store.add_item(item.clone());
        for pinned in [false, true] {
            ui.app.store.update(&item.id, |item| item.pinned = pinned);
            ui.refresh();
            let frame = ui.render_frame(W as u32, H as u32);
            let pixel = frame.pixel(1060,35).unwrap();
            assert_eq!((pixel.red(),pixel.green(),pixel.blue()), (16,18,28), "removed toolbar button still visible");
            assert!(!ui.hits.iter().any(|(x,y,w,h,_)| *x<=1060.0 && 1060.0<*x+*w && *y<=35.0 && 35.0<*y+*h), "removed button still clickable");
            assert!(ui.hits.iter().any(|(_,_,_,_,hit)| *hit==Hit::Pin(0)));
        }
    }

    #[test]
    fn keyboard_hint_bar_has_its_own_visible_background() {
        let mut ui = fixture_ui();
        let frame = ui.render_frame(W as u32, H as u32);
        assert!(frame.pixel(210,340).unwrap().red() > 20, "keyboard hints lack a visible footer band");
    }

    #[test]
    fn unsupported_privacy_controls_cannot_be_clicked_or_toggled() {
        let mut ui = fixture_ui();
        ui.settings = true;
        for wayland in [true, false] {
            ui.wayland_session = wayland;
            ui.render_frame(W as u32, H as u32);
            for key in ["transient", "share", "link_previews", "confidential"] {
                if key == "confidential" && !wayland { continue; }
                assert!(!ui.hits.iter().any(|(_,_,_,_,hit)| *hit == Hit::Toggle(key)), "unsupported {key} has a toggle");
                let before = ui.app.store.settings.clone();
                ui.apply(AppEvent::ToggleSetting { key });
                assert_eq!(ui.app.store.settings, before, "unsupported {key} preference changed");
            }
            assert!(ui.hits.iter().any(|(_,_,_,_,hit)| *hit == Hit::Toggle("watch")));
            if !wayland { assert!(ui.hits.iter().any(|(_,_,_,_,hit)| *hit == Hit::Toggle("confidential"))); }
        }
    }

    #[test]
    fn global_shortcut_setup_offers_copy_command_and_system_settings() {
        let mut ui = fixture_ui();
        ui.settings = true;
        ui.render_frame(W as u32, H as u32);
        for action in [Hit::OpenKeyboardSettings, Hit::CopyShortcutCommand] {
            assert!(ui.hits.iter().any(|(_,_,_,_,hit)| *hit == action), "shortcut setup action missing: {action:?}");
        }
    }

    #[test]
    fn shortcut_copy_queues_the_running_executable_command() {
        let (mut ui, worker) = fixture_ui_with_worker();
        ui.settings = true;
        ui.render_frame(W as u32, H as u32);
        click_hit(&mut ui, Hit::CopyShortcutCommand);
        let WorkerCmd::Write { payload, purpose } = worker.try_recv().expect("Copy Command did not queue a clipboard write") else { panic!("expected clipboard write") };
        let crate::events::WritePayload::Text(command) = payload else { panic!("shortcut command is not text") };
        assert_eq!(purpose, WritePurpose::Copy);
        assert!(command.contains(std::env::current_exe().unwrap().to_string_lossy().as_ref()));
        assert!(command.ends_with(" toggle"));
        assert!(ui.settings);
        assert!(ui.app.store.items.is_empty());
    }

    #[test]
    fn keyboard_settings_launcher_opens_only_the_keyboard_panel() {
        let command = keyboard_settings_command();
        assert_eq!(command.get_program(), "gnome-control-center");
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["keyboard"]);
    }

    #[test]
    fn drop_confirmation_animates_then_returns_to_idle() {
        let mut ui = fixture_ui();
        let now = std::time::Instant::now();
        ui.visible = true;
        ui.drop_feedback = Some(DropFeedback { board_id: "target".into(), started: now });
        let feedback = ui.drop_feedback.as_ref().unwrap();
        assert_eq!(feedback.strength(now), 1.0);
        assert!(feedback.strength(now + std::time::Duration::from_millis(400)) < 1.0);
        assert!(ui.next_animation_deadline(now).is_some());
        ui.advance_drop_animation(now + std::time::Duration::from_millis(850));
        assert!(ui.drop_feedback.is_none());
        assert!(ui.next_animation_deadline(now + std::time::Duration::from_millis(850)).is_none());
    }

    #[test]
    fn main_window_requests_always_on_top() {
        assert_eq!(window_attributes().window_level, winit::window::WindowLevel::AlwaysOnTop);
    }

    #[test]
    fn board_removal_can_explicitly_delete_from_history_too() {
        let mut ui = fixture_ui();
        let board = ui.app.store.add_pinboard("Board");
        let mut item = fixture_item(clipdeck_core::ItemType::Text, "Delete only after opting in");
        item.pinboard_id = Some(board.id);
        ui.app.store.add_item(item.clone());
        ui.filter = 2;
        ui.refresh();
        ui.render_frame(W as u32, H as u32);
        click_hit(&mut ui, Hit::Delete(0));
        ui.render_frame(W as u32, H as u32);
        click_hit(&mut ui, Hit::ToggleDeleteHistory);
        assert!(ui.app.store.find(&item.id).is_some(), "checkbox deleted before confirmation");
        ui.render_frame(W as u32, H as u32);
        click_hit(&mut ui, Hit::ConfirmDelete);
        assert!(ui.app.store.find(&item.id).is_none());
        let saved = clipdeck_core::Store::open(ui.app.store.image_dir.parent().unwrap());
        assert!(saved.find(&item.id).is_none());
    }

    #[test]
    fn removing_clip_from_board_preserves_history_by_default() {
        let mut ui = fixture_ui();
        let board = ui.app.store.add_pinboard("Board");
        let mut item = fixture_item(clipdeck_core::ItemType::Text, "Keep in History");
        item.pinboard_id = Some(board.id.clone());
        ui.app.store.add_item(item.clone());
        ui.filter = 2;
        ui.refresh();
        ui.render_frame(W as u32, H as u32);
        click_hit(&mut ui, Hit::Delete(0));
        ui.render_frame(W as u32, H as u32);
        click_hit(&mut ui, Hit::ConfirmDelete);
        let kept = ui.app.store.find(&item.id).expect("board removal deleted History");
        assert!(kept.pinboard_id.is_none());
        assert_eq!(kept.content, item.content);
        assert!(ui.items.is_empty());
        let saved = clipdeck_core::Store::open(ui.app.store.image_dir.parent().unwrap());
        assert!(saved.find(&item.id).unwrap().pinboard_id.is_none());
    }

    #[test]
    fn delete_clip_requires_confirmation_and_can_be_cancelled() {
        let mut ui = fixture_ui();
        let item = fixture_item(clipdeck_core::ItemType::Text, "Do not delete without consent");
        ui.app.store.add_item(item.clone());
        ui.refresh();
        card_frame(&mut ui, &item);
        click_hit(&mut ui, Hit::Delete(0));
        assert!(ui.app.store.find(&item.id).is_some(), "first delete click destroyed the clip");
        assert!(ui.dialog.is_some());
        ui.render_frame(W as u32, H as u32);
        click_hit(&mut ui, Hit::CancelDialog);
        assert_eq!(ui.app.store.find(&item.id), Some(&item));
    }

    #[test]
    fn delete_board_requires_confirmation_and_can_be_cancelled() {
        let mut ui = fixture_ui();
        let board = ui.app.store.add_pinboard("Keep this board");
        ui.refresh();
        ui.render_frame(W as u32, H as u32);
        click_hit(&mut ui, Hit::DeletePinboard(0));
        assert_eq!(ui.app.store.pinboards.len(), 1, "first delete click destroyed the board");
        assert!(ui.dialog.is_some());
        ui.render_frame(W as u32, H as u32);
        click_hit(&mut ui, Hit::CancelDialog);
        assert_eq!(ui.app.store.pinboards[0].id, board.id);
    }

    #[test]
    fn active_drag_has_a_floating_preview_without_new_hit_regions() {
        let mut ui = fixture_ui();
        let item = fixture_item(clipdeck_core::ItemType::Text, "Floating drag preview");
        ui.app.store.add_item(item.clone());
        ui.refresh();
        ui.drag = Some(CardDrag { id: item.id, start: (250.0, 160.0), active: true, board_id: None });
        ui.mouse = Some((500.0, 160.0));
        let mut preview = Pixmap::new(W as u32, H as u32).unwrap();
        ui.draw_drag_preview(&mut preview, W, H);
        assert!(preview.pixel(520, 180).unwrap().alpha() > 0, "dragged card has no floating preview");
        assert!(ui.hits.is_empty(), "floating preview intercepts drop targets");
        assert_eq!(preview.pixel(0, 0).unwrap().alpha(), 0);
    }

    #[test]
    fn dragging_card_to_sidebar_board_moves_by_id_without_clipboard_write() {
        let (mut ui, worker) = fixture_ui_with_worker();
        let board = ui.app.store.add_pinboard("Drop target");
        let item = fixture_item(clipdeck_core::ItemType::Text, "Dragged clip");
        ui.app.store.add_item(item.clone());
        ui.refresh();
        ui.render_frame(W as u32, H as u32);
        ui.pointer_moved(250.0, 160.0);
        ui.pointer_pressed();
        // Clipboard captures may reorder history while a drag is in progress.
        let mut newer = fixture_item(clipdeck_core::ItemType::Text, "New capture");
        newer.id = "new-capture".into();
        ui.app.store.add_item(newer.clone());
        ui.refresh();
        ui.pointer_moved(156.0, 125.0); // Even over the board's delete icon.
        let preview = ui.render_frame(W as u32, H as u32);
        let target = preview.pixel(25, 120).unwrap();
        assert!(target.blue() > target.red() + 15, "drop target did not highlight");
        ui.pointer_released();
        assert_eq!(ui.app.store.find(&item.id).unwrap().pinboard_id.as_deref(), Some(board.id.as_str()));
        assert!(ui.app.store.find(&newer.id).unwrap().pinboard_id.is_none());
        assert_eq!(ui.app.store.pinboards.len(), 1, "drop clicked a board icon");
        assert_eq!(ui.drop_feedback.as_ref().map(|feedback| feedback.board_id.as_str()), Some(board.id.as_str()), "successful drop has no confirmation");
        assert!(worker.try_recv().is_err(), "drag wrote the clipboard");
        let saved = clipdeck_core::Store::open(ui.app.store.image_dir.parent().unwrap());
        assert_eq!(saved.find(&item.id).unwrap().pinboard_id.as_deref(), Some(board.id.as_str()));
    }

    #[test]
    fn releasing_drag_outside_board_does_not_move_or_activate_clip() {
        let (mut ui, worker) = fixture_ui_with_worker();
        ui.app.store.add_pinboard("Do not select");
        let item = fixture_item(clipdeck_core::ItemType::Text, "Cancel drag");
        ui.app.store.add_item(item.clone());
        ui.refresh();
        ui.render_frame(W as u32, H as u32);
        ui.pointer_moved(250.0, 160.0);
        ui.pointer_pressed();
        ui.pointer_moved(-20.0, 150.0);
        ui.pointer_released();
        assert_eq!(ui.app.store.find(&item.id), Some(&item));
        assert!(worker.try_recv().is_err());
    }

    #[test]
    fn pointer_click_still_selects_and_double_click_still_copies() {
        let (mut ui, worker) = fixture_ui_with_worker();
        let item = fixture_item(clipdeck_core::ItemType::Text, "Pointer clicks");
        ui.app.store.add_item(item.clone());
        ui.refresh();
        ui.render_frame(W as u32, H as u32);
        ui.pointer_moved(250.0, 160.0);
        ui.pointer_pressed();
        ui.pointer_released();
        assert!(worker.try_recv().is_err());
        ui.pointer_pressed();
        assert!(worker.try_recv().is_err(), "copy activated before pointer release");
        ui.pointer_released();
        let WorkerCmd::Write { purpose, .. } = worker.try_recv().expect("double click stopped copying") else { panic!("expected clipboard write") };
        assert_eq!(purpose, WritePurpose::Copy);
    }

    #[test]
    fn main_frame_has_transparent_rounded_corners_and_opaque_content() {
        let mut ui = fixture_ui();
        let pm = ui.render_frame(W as u32, H as u32);
        for (x, y) in [(0, 0), (1239, 0), (0, 371), (1239, 371), (7, 7), (1232, 364)] {
            assert_eq!(pm.pixel(x, y).unwrap().alpha(), 0, "opaque window corner at {x},{y}");
        }
        assert_eq!(pm.pixel(620, 186).unwrap().alpha(), 255);
        ui.dialog = Some(Dialog::Rename { id: "test".into(), draft: "Name".into(), select_all: true });
        let modal = ui.render_frame(W as u32, H as u32);
        assert_eq!(modal.pixel(7, 7).unwrap().alpha(), 0, "dialog filled rounded window corner");
    }

    #[test]
    fn rename_board_preserves_membership_and_persists() {
        let mut ui = fixture_ui();
        let board = ui.app.store.add_pinboard("Old board");
        let mut item = fixture_item(clipdeck_core::ItemType::Text, "Keep membership");
        item.pinboard_id = Some(board.id.clone());
        ui.app.store.add_item(item.clone());
        ui.refresh();
        let mut pm = Pixmap::new(W as u32, H as u32).unwrap();
        ui.layout(&mut pm, W, H);
        ui.mouse = Some((132.0, 125.0));
        ui.click();
        assert!(matches!(ui.dialog, Some(Dialog::Rename { .. })), "board pencil did not open rename");
        ui.handle_key_action(KeyAction::QueryPush("Work notes".into()));
        ui.handle_key_action(KeyAction::Activate { plain: false });
        assert_eq!(ui.app.store.pinboards[0].name, "Work notes");
        assert_eq!(ui.app.store.pinboards[0].id, board.id);
        assert_eq!(ui.app.store.find(&item.id).unwrap().pinboard_id.as_deref(), Some(board.id.as_str()));
        assert!(ui.dialog.is_none());
        assert!(ui.query.is_empty(), "rename keystrokes leaked into search");
        let saved = clipdeck_core::Store::open(ui.app.store.image_dir.parent().unwrap());
        assert_eq!(saved.pinboards[0].name, "Work notes");
    }

    #[test]
    fn move_card_to_another_board_and_back_to_history() {
        let mut ui = fixture_ui();
        let first = ui.app.store.add_pinboard("First");
        let second = ui.app.store.add_pinboard("Second");
        let mut item = fixture_item(clipdeck_core::ItemType::Text, "Move only membership");
        item.pinboard_id = Some(first.id.clone());
        item.pinned = true;
        item.stack_order = Some(1);
        ui.app.store.add_item(item.clone());
        ui.refresh();
        ui.filter = 2;
        ui.refresh();
        card_frame(&mut ui, &item);
        ui.mouse = Some((235.0, 77.0));
        ui.click();
        assert!(matches!(ui.dialog, Some(Dialog::Move { .. })), "card move control did not open board picker");
        let mut pm = Pixmap::new(W as u32, H as u32).unwrap();
        ui.layout(&mut pm, W, H);
        click_hit(&mut ui, Hit::ChoosePinboard(2));
        let moved = ui.app.store.find(&item.id).unwrap();
        assert_eq!(moved.pinboard_id.as_deref(), Some(second.id.as_str()));
        assert!(moved.pinned);
        assert_eq!(moved.stack_order, Some(1));
        assert_eq!(moved.content, item.content);
        assert!(ui.items.is_empty(), "moved item remained in its old board view");
        assert!(ui.dialog.is_none());
        ui.filter = 0;
        ui.refresh();
        ui.hits.clear();
        card_frame(&mut ui, &item);
        click_hit(&mut ui, Hit::MoveToBoard(0));
        ui.layout(&mut pm, W, H);
        click_hit(&mut ui, Hit::ChoosePinboard(0));
        let saved = clipdeck_core::Store::open(ui.app.store.image_dir.parent().unwrap());
        assert!(saved.find(&item.id).unwrap().pinboard_id.is_none());
        assert!(saved.find(&item.id).unwrap().pinned);
    }

    #[test]
    fn blank_board_name_is_rejected_and_cancel_preserves_name() {
        let mut ui = fixture_ui();
        let board = ui.app.store.add_pinboard("Original");
        ui.refresh();
        ui.dialog = Some(Dialog::Rename { id: board.id.clone(), draft: board.name.clone(), select_all: true });
        ui.handle_key_action(KeyAction::QueryPush("   ".into()));
        ui.handle_key_action(KeyAction::Activate { plain: false });
        assert_eq!(ui.app.store.pinboards[0].name, "Original");
        assert!(ui.dialog.is_some());
        assert!(ui.query.is_empty());
        let mut pm = Pixmap::new(W as u32, H as u32).unwrap();
        ui.layout(&mut pm, W, H);
        assert!(!ui.hits.iter().any(|(_, _, _, _, hit)| matches!(hit, Hit::DeletePinboard(_) | Hit::SetFilter(_))), "dialog allowed sidebar click-through");
        click_hit(&mut ui, Hit::CancelDialog);
        assert!(ui.dialog.is_none());
        assert_eq!(ui.app.store.pinboards[0].name, "Original");
    }

    #[test]
    fn board_picker_scroll_reaches_last_board() {
        let mut ui = fixture_ui();
        for n in 0..13 { ui.app.store.add_pinboard(&format!("Board {n}")); }
        let item = fixture_item(clipdeck_core::ItemType::Text, "Scrollable picker");
        ui.app.store.add_item(item.clone());
        ui.refresh();
        ui.dialog = Some(Dialog::Move { item_id: item.id.clone(), scroll: 0 });
        for _ in 0..20 { ui.scroll_board_picker(-60.0, H); }
        let mut pm = Pixmap::new(W as u32, H as u32).unwrap();
        ui.layout(&mut pm, W, H);
        click_hit(&mut ui, Hit::ChoosePinboard(13));
        assert_eq!(ui.app.store.find(&item.id).unwrap().pinboard_id.as_deref(), Some(ui.pinboards[12].id.as_str()));
    }

    #[test]
    fn missing_destination_does_not_change_clip_membership() {
        let mut ui = fixture_ui();
        let board = ui.app.store.add_pinboard("Keep membership");
        let mut item = fixture_item(clipdeck_core::ItemType::Text, "Unchanged");
        item.pinboard_id = Some(board.id.clone());
        ui.app.store.add_item(item.clone());
        let generation = ui.app.generation;
        ui.app.move_to_pinboard(&item.id, Some("missing-board"));
        assert_eq!(ui.app.store.find(&item.id), Some(&item));
        assert_eq!(ui.app.generation, generation);
    }

    #[test]
    fn enter_copies_without_requesting_simulated_paste() {
        let (mut ui, worker) = fixture_ui_with_worker();
        let item = fixture_item(clipdeck_core::ItemType::Text, "Synthetic activation");
        ui.app.store.add_item(item.clone());
        ui.refresh();
        ui.visible = true;
        ui.handle_key_action(route_key(&WinitKey::Named(NamedKey::Enter), false, false));
        let WorkerCmd::Write { payload, purpose } = worker.try_recv().expect("Enter did not copy") else { panic!("expected clipboard write") };
        assert_eq!(purpose, WritePurpose::Copy, "Enter requested paste/input simulation");
        assert!(matches!(payload, crate::events::WritePayload::Text(ref text) if text == &item.content));
        assert!(!ui.visible);
    }

    #[test]
    fn double_click_copies_the_clicked_card_and_closes() {
        let (mut ui, worker) = fixture_ui_with_worker();
        let item = fixture_item(clipdeck_core::ItemType::Text, "Synthetic double click");
        ui.app.store.add_item(item.clone());
        ui.refresh();
        ui.visible = true;
        card_frame(&mut ui, &item);
        ui.mouse = Some((240.0, 150.0));
        ui.click();
        assert!(worker.try_recv().is_err(), "single click should only select");
        ui.click();
        let WorkerCmd::Write { payload, purpose } = worker.try_recv().expect("double click did not copy") else { panic!("expected clipboard write") };
        assert_eq!(purpose, WritePurpose::Copy);
        assert!(matches!(payload, crate::events::WritePayload::Text(ref text) if text == &item.content));
        assert!(!ui.visible);
    }

    #[test]
    fn escape_closes_settings_and_clears_interaction_state() {
        let mut ui = fixture_ui();
        ui.visible = true;
        ui.settings = true;
        ui.capture = true;
        ui.editing = Some(("qa".into(), "draft".into()));
        ui.hits.push((0.0, 0.0, 20.0, 20.0, Hit::CloseWindow));
        ui.handle_key_action(KeyAction::Close);
        assert!(!ui.visible, "Escape only switched settings to history");
        assert!(!ui.settings && !ui.capture && ui.editing.is_none());
        assert!(ui.hits.is_empty());
    }

    #[test]
    fn close_button_clears_stale_hit_regions() {
        let mut ui = fixture_ui();
        ui.visible = true;
        let mut pm = Pixmap::new(W as u32, H as u32).unwrap();
        ui.layout(&mut pm, W, H);
        ui.mouse = Some((W - 69.0, 35.0));
        ui.click();
        assert!(!ui.visible);
        assert!(ui.hits.is_empty(), "closed overlay retained clickable regions");
    }

    #[test]
    fn deleting_pinboard_keeps_its_clips_and_returns_to_history() {
        let mut ui = fixture_ui();
        let board = ui.app.store.add_pinboard("Synthetic board");
        let mut item = fixture_item(clipdeck_core::ItemType::Text, "Keep this clip");
        item.pinboard_id = Some(board.id.clone());
        ui.app.store.add_item(item.clone());
        ui.refresh();
        ui.filter = 2;
        let mut pm = Pixmap::new(W as u32, H as u32).unwrap();
        ui.layout(&mut pm, W, H);
        ui.mouse = Some((156.0, 125.0));
        ui.click();
        assert_eq!(ui.app.store.pinboards.len(), 1, "board deleted before confirmation");
        ui.render_frame(W as u32, H as u32);
        click_hit(&mut ui, Hit::ConfirmDelete);
        assert!(ui.app.store.pinboards.is_empty(), "delete board control did not remove the board");
        assert_eq!(ui.filter, 0);
        assert!(ui.app.store.find(&item.id).unwrap().pinboard_id.is_none());
        let saved = clipdeck_core::Store::open(ui.app.store.image_dir.parent().unwrap());
        assert!(saved.pinboards.is_empty());
        assert!(saved.find(&item.id).is_some());
    }

    #[test]
    fn window_uses_bundled_application_icon() {
        assert!(window_attributes().window_icon.is_some(), "window has no application icon");
    }

    #[test]
    fn settings_command_opens_settings_instead_of_history() {
        let mut ui = fixture_ui();
        ui.apply(AppEvent::OpenSettings);
        assert!(ui.visible);
        assert!(ui.settings, "show_overlay reset the settings command");
    }

    #[test]
    fn wayland_settings_offer_desktop_binding_instead_of_x11_recorder() {
        let mut ui = fixture_ui();
        let mut pm = Pixmap::new(W as u32, H as u32).unwrap();
        ui.wayland_session = true;
        ui.draw_settings(&mut pm, W, H);
        assert!(!ui.hits.iter().any(|(_, _, _, _, hit)| *hit == Hit::ShortcutCapture), "Wayland advertised unsupported global shortcut capture");
        ui.hits.clear();
        ui.wayland_session = false;
        ui.draw_settings(&mut pm, W, H);
        assert!(ui.hits.iter().any(|(_, _, _, _, hit)| *hit == Hit::ShortcutCapture), "X11 recorder disappeared");
    }

    #[test]
    fn new_button_creates_and_selects_a_persisted_pinboard() {
        let mut ui = fixture_ui();
        let mut pm = Pixmap::new(W as u32, H as u32).unwrap();
        ui.layout(&mut pm, W, H);
        ui.mouse = Some((40.0, H - 30.0));
        ui.click();
        assert_eq!(ui.app.store.pinboards.len(), 1, "+ New did not create a pinboard");
        assert_eq!(ui.filter, 2);
        let root = ui.app.store.image_dir.parent().unwrap();
        let saved = clipdeck_core::Store::open(root);
        assert_eq!(saved.pinboards, ui.app.store.pinboards);
    }

    #[test]
    fn card_icon_hits_win_over_the_card_body() {
        let mut ui = fixture_ui();
        let item = fixture_item(clipdeck_core::ItemType::Text, "Synthetic card");
        ui.app.store.add_item(item.clone());
        ui.refresh();
        card_frame(&mut ui, &item);
        ui.mouse = Some((363.0, 77.0));
        ui.click();
        assert!(ui.app.store.find(&item.id).unwrap().pinned, "card body shadowed pin icon");
        ui.mouse = Some((387.0, 77.0));
        ui.click();
        assert_eq!(ui.editing.as_ref().map(|(id, _)| id.as_str()), Some(item.id.as_str()), "card body shadowed edit icon");
        ui.mouse = Some((409.0, 77.0));
        ui.click();
        assert!(ui.app.store.find(&item.id).is_some(), "clip deleted before confirmation");
        ui.render_frame(W as u32, H as u32);
        click_hit(&mut ui, Hit::ConfirmDelete);
        assert!(ui.app.store.find(&item.id).is_none(), "card body shadowed delete icon");
    }

    #[test]
    fn scrolling_keeps_partial_cards_visible_and_hits_out_of_sidebar() {
        let mut ui = fixture_ui();
        ui.items = (0..9).map(|_| fixture_item(clipdeck_core::ItemType::Text, &"Long synthetic line. ".repeat(100))).collect();
        for offset in [100.0, 220.0, 300.0, 460.0] {
            ui.scroll_x = offset;
            ui.hits.clear();
            let mut pm = Pixmap::new(W as u32, H as u32).unwrap();
            pm.fill(Color::from_rgba8(16, 18, 28, 255));
            ui.layout(&mut pm, W, H);
            let background = tiny_skia::PremultipliedColorU8::from_rgba(16, 18, 28, 255).unwrap();
            assert_ne!(pm.pixel(200, 200), Some(background), "partial card disappeared at offset {offset}");
            for (x, _, w, _, hit) in &ui.hits {
                if matches!(hit, Hit::Select(_) | Hit::Pin(_) | Hit::Edit(_) | Hit::Delete(_)) && *w > 0.0 {
                    assert!(*x >= 196.0, "card hit leaked into sidebar");
                }
            }
        }
    }

    #[test]
    fn card_content_cannot_paint_outside_its_rectangle() {
        let mut ui = fixture_ui();
        let item = fixture_item(clipdeck_core::ItemType::File, &"W".repeat(300));
        let pm = card_frame(&mut ui, &item);
        let background = tiny_skia::PremultipliedColorU8::from_rgba(16, 18, 28, 255).unwrap();
        for y in 0..pm.height() {
            for x in 0..pm.width() {
                if !(196..424).contains(&x) || !(64..316).contains(&y) {
                    assert_eq!(pm.pixel(x, y), Some(background), "card ink escaped at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn image_redraw_reuses_thumbnail_until_store_changes() {
        let mut ui = fixture_ui();
        let path = ui.app.store.image_dir.join("qa.png");
        std::fs::write(&path, include_bytes!("../assets/icon.png")).unwrap();
        let mut item = fixture_item(clipdeck_core::ItemType::Image, "qa.png");
        item.image_file = Some("qa.png".into());
        let first = card_frame(&mut ui, &item);
        std::fs::remove_file(&path).unwrap();
        let second = card_frame(&mut ui, &item);
        assert!(first.data() == second.data(), "redraw re-read the image file");
        ui.app.generation += 1;
        let invalidated = card_frame(&mut ui, &item);
        assert!(first.data() != invalidated.data(), "store change did not invalidate the image");
    }

    fn ch(c: &str) -> WinitKey {
        WinitKey::Character(c.into())
    }

    #[test]
    fn arrow_left_without_shift_goes_prev() {
        assert_eq!(route_key(&WinitKey::Named(NamedKey::ArrowLeft), false, false), KeyAction::Prev);
    }

    #[test]
    fn arrow_right_goes_next() {
        assert_eq!(route_key(&WinitKey::Named(NamedKey::ArrowRight), false, false), KeyAction::Next);
    }

    #[test]
    fn tab_navigates_shift_reverses() {
        assert_eq!(route_key(&WinitKey::Named(NamedKey::Tab), false, false), KeyAction::Next);
        assert_eq!(route_key(&WinitKey::Named(NamedKey::Tab), false, true), KeyAction::Prev);
    }

    #[test]
    fn plain_letters_type_into_search() {
        for c in ["e", "s", "f", "a"] {
            assert_eq!(route_key(&ch(c), false, false), KeyAction::QueryPush(c.into()), "letter {c}");
        }
    }

    #[test]
    fn shifted_letter_types_into_search() {
        assert_eq!(route_key(&ch("S"), false, true), KeyAction::QueryPush("S".into()));
    }

    #[test]
    fn ctrl_shortcuts_still_work() {
        assert_eq!(route_key(&ch("c"), true, false), KeyAction::CopySelected);
        assert_eq!(route_key(&ch("s"), true, false), KeyAction::StackSelected);
        assert_eq!(route_key(&ch("e"), true, false), KeyAction::EditSelected);
    }

    #[test]
    fn enter_activates_shift_enters_plain() {
        assert_eq!(route_key(&WinitKey::Named(NamedKey::Enter), false, false), KeyAction::Activate { plain: false });
        assert_eq!(route_key(&WinitKey::Named(NamedKey::Enter), false, true), KeyAction::Activate { plain: true });
    }

    #[test]
    fn backspace_pops_query_delete_removes_item() {
        assert_eq!(route_key(&WinitKey::Named(NamedKey::Backspace), false, false), KeyAction::QueryPop);
        assert_eq!(route_key(&WinitKey::Named(NamedKey::Delete), false, false), KeyAction::DeleteSelected);
    }

    #[test]
    fn escape_closes() {
        assert_eq!(route_key(&WinitKey::Named(NamedKey::Escape), false, false), KeyAction::Close);
    }
}
