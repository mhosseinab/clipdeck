//! System tray (StatusNotifierItem) via `ksni` — pure Rust, no system libs.
//! On Ubuntu/GNOME the AppIndicator extension surfaces it; where no SNI host
//! exists the tray silently stays absent and the CLI/shortcut remain the
//! control surfaces.

use crate::state::AppState;
use crate::ui::Command;
use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem, Tray};
use std::sync::{Arc, Mutex};
use winit::event_loop::EventLoopProxy;

pub struct ClipTray {
    pub state: Arc<Mutex<AppState>>,
    pub proxy: EventLoopProxy<Command>,
}

/// 22×22 ARGB32 icon matching the app's design language (rounded dark tile,
/// indigo tab, white card) — drawn in code, no external asset.
pub fn tray_icon() -> Icon {
    const S: i32 = 22;
    let mut data = vec![0u8; (S * S * 4) as usize];
    let put = |data: &mut Vec<u8>, x: i32, y: i32, a: u8, r: u8, g: u8, b: u8| {
        if x < 0 || y < 0 || x >= S || y >= S {
            return;
        }
        let o = ((y * S + x) * 4) as usize;
        data[o] = a;
        data[o + 1] = r;
        data[o + 2] = g;
        data[o + 3] = b;
    };
    for y in 1..S - 1 {
        for x in 1..S - 1 {
            let corner = (x < 4 || x >= S - 3) && (y < 4 || y >= S - 3);
            if corner && x != 3 && x != S - 4 && y != 3 && y != S - 4 {
                continue;
            }
            put(&mut data, x, y, 255, 24, 26, 38);
        }
    }
    for y in 3..6 {
        for x in 8..13 {
            put(&mut data, x, y, 255, 122, 162, 255);
        }
    }
    for y in 8..18 {
        for x in 4..17 {
            put(&mut data, x, y, 255, 235, 238, 246);
        }
    }
    for y in 10..16 {
        for x in 6..14 {
            if y % 2 == 0 {
                put(&mut data, x, y, 255, 140, 146, 166);
            }
        }
    }
    Icon { width: S, height: S, data }
}

impl Tray for ClipTray {
    fn id(&self) -> String {
        "dev.clipdeck.app".into()
    }

    fn title(&self) -> String {
        "ClipDeck".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        vec![tray_icon()]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.proxy.send_event(Command::Show).ok();
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let watching = self.state.lock().map(|s| s.store.settings.watch_enabled).unwrap_or(true);
        let recents: Vec<(String, String)> = self
            .state
            .lock()
            .map(|s| {
                s.store
                    .recent(6)
                    .into_iter()
                    .map(|i| {
                        let label = if i.title.is_empty() { i.preview.clone() } else { i.title.clone() };
                        (i.id.clone(), label.chars().take(42).collect::<String>())
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut items: Vec<MenuItem<Self>> = vec![
            MenuItem::Standard(StandardItem {
                label: "Open ClipDeck".into(),
                activate: Box::new(|t| {
                    t.proxy.send_event(Command::Show).ok();
                }),
                ..Default::default()
            }),
            MenuItem::Standard(StandardItem {
                label: "Settings…".into(),
                activate: Box::new(|t| {
                    t.proxy.send_event(Command::OpenSettings).ok();
                }),
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: if watching { "Pause watching" } else { "Resume watching" }.into(),
                activate: Box::new(|t| {
                    if let Ok(mut st) = t.state.lock() {
                        let on = !st.store.settings.watch_enabled;
                        st.store.settings.watch_enabled = on;
                        st.store.save();
                    }
                    t.proxy.send_event(Command::Refresh).ok();
                }),
                ..Default::default()
            }),
        ];

        if !recents.is_empty() {
            items.push(MenuItem::Separator);
            for (id, label) in recents {
                items.push(MenuItem::Standard(StandardItem {
                    label,
                    activate: Box::new(move |t| {
                        if let Ok(mut st) = t.state.lock() {
                            st.copy_item(&id);
                        }
                        crate::sounds::play("copy");
                    }),
                    ..Default::default()
                }));
            }
        }

        items.push(MenuItem::Separator);
        items.push(MenuItem::Standard(StandardItem {
            label: "Quit".into(),
            activate: Box::new(|t| {
                    t.proxy.send_event(Command::Quit).ok();
                }),
            ..Default::default()
        }));
        items
    }
}

/// Spawn the tray when a session bus exists; silently skip otherwise (e.g. a
/// headless session) so the daemon still runs.
pub fn spawn(state: Arc<Mutex<AppState>>, proxy: EventLoopProxy<Command>) {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        return;
    }
    let service = ksni::TrayService::new(ClipTray { state, proxy });
    service.spawn();
}
