//! System tray (StatusNotifierItem) via `ksni`. The tray SENDS `AppEvent`s;
//! display state arrives as a read-only `TrayView` mirror (single writer:
//! main thread; readers: dbus thread) — never held across code.

use crate::events::{AppEvent, WritePurpose};
use clipdeck_core::ClipItem;
use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem, Tray};
use std::sync::{Arc, RwLock};
use winit::event_loop::EventLoopProxy;

#[derive(Default)]
pub struct TrayView {
    pub watching: bool,
    pub recent: Vec<(String, String)>,
}

pub struct ClipTray {
    pub view: Arc<RwLock<TrayView>>,
    pub events: EventLoopProxy<AppEvent>,
}

/// Supplied tray PNG converted from straight RGBA to ARGB32 network byte order.
pub fn tray_icon() -> Icon {
    let (width, height, rgba) = crate::worker::decode_png(include_bytes!("../assets/icon_tray.png"))
        .expect("bundled tray PNG");
    let data = rgba.chunks_exact(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect();
    Icon { width: width as i32, height: height as i32, data }
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
        self.events.send_event(AppEvent::Show).ok();
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let view = self.view.read().map(|v| (v.watching, v.recent.clone())).unwrap_or((true, vec![]));
        let (watching, recents) = view;

        let mut items: Vec<MenuItem<Self>> = vec![
            MenuItem::Standard(StandardItem {
                label: "Open ClipDeck".into(),
                activate: Box::new(|t| {
                    t.events.send_event(AppEvent::Show).ok();
                }),
                ..Default::default()
            }),
            MenuItem::Standard(StandardItem {
                label: "Settings…".into(),
                activate: Box::new(|t| {
                    t.events.send_event(AppEvent::OpenSettings).ok();
                }),
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: if watching { "Pause watching" } else { "Resume watching" }.into(),
                activate: Box::new(|t| {
                    t.events.send_event(AppEvent::ToggleWatch).ok();
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
                        t.events.send_event(AppEvent::Copy { id: id.clone(), purpose: WritePurpose::TrayCopy }).ok();
                    }),
                    ..Default::default()
                }));
            }
        }

        items.push(MenuItem::Separator);
        items.push(MenuItem::Standard(StandardItem {
            label: "Quit".into(),
            activate: Box::new(|t| {
                    t.events.send_event(AppEvent::Quit).ok();
                }),
            ..Default::default()
        }));
        items
    }
}

/// Update the tray's read-only mirror after state changes (main thread only).
pub fn refresh_view(view: &Arc<RwLock<TrayView>>, watching: bool, items: &[ClipItem]) {
    if let Ok(mut v) = view.write() {
        v.watching = watching;
        v.recent = items
            .iter()
            .take(6)
            .map(|i| {
                let label = if i.title.is_empty() { i.preview.clone() } else { i.title.clone() };
                (i.id.clone(), label.chars().take(42).collect::<String>())
            })
            .collect();
    }
}

/// Spawn the tray when a session bus exists; silently skip otherwise.
pub fn spawn(view: Arc<RwLock<TrayView>>, events: EventLoopProxy<AppEvent>) {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        return;
    }
    ksni::TrayService::new(ClipTray { view, events }).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_pixels_match_supplied_png_in_argb_network_order() {
        let (width, height, rgba) = crate::worker::decode_png(include_bytes!("../assets/icon_tray.png")).unwrap();
        let icon = tray_icon();
        assert_eq!((icon.width, icon.height), (width as i32, height as i32));
        let expected: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect();
        assert!(icon.data == expected, "tray still uses substitute pixels or incorrect byte order");
    }
}
