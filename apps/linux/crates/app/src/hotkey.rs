//! Global shortcut: ONE thread owns the `GlobalHotKeyManager` (sole owner of
//! X11 grabs). Presses become `AppEvent::Toggle`; re-registrations arrive on a
//! command channel. Registration failure degrades gracefully (CLI works).

use crate::events::AppEvent;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager};
use std::sync::mpsc::Receiver;
use std::time::Duration;
use winit::event_loop::EventLoopProxy;

/// Parse our canonical accelerator ("CommandOrControl+Shift+V") into a HotKey.
pub fn parse_hotkey(accel: &str) -> Option<HotKey> {
    let mut mods = Modifiers::empty();
    let mut code: Option<Code> = None;
    for part in accel.split('+') {
        match part {
            "CommandOrControl" | "Control" | "Command" => mods |= Modifiers::CONTROL,
            "Alt" => mods |= Modifiers::ALT,
            "Shift" => mods |= Modifiers::SHIFT,
            key => code = Some(code_from_key(key)?),
        }
    }
    Some(HotKey::new(Some(mods), code?))
}

fn code_from_key(key: &str) -> Option<Code> {
    Some(match key {
        "A" => Code::KeyA, "B" => Code::KeyB, "C" => Code::KeyC, "D" => Code::KeyD,
        "E" => Code::KeyE, "F" => Code::KeyF, "G" => Code::KeyG, "H" => Code::KeyH,
        "I" => Code::KeyI, "J" => Code::KeyJ, "K" => Code::KeyK, "L" => Code::KeyL,
        "M" => Code::KeyM, "N" => Code::KeyN, "O" => Code::KeyO, "P" => Code::KeyP,
        "Q" => Code::KeyQ, "R" => Code::KeyR, "S" => Code::KeyS, "T" => Code::KeyT,
        "U" => Code::KeyU, "V" => Code::KeyV, "W" => Code::KeyW, "X" => Code::KeyX,
        "Y" => Code::KeyY, "Z" => Code::KeyZ,
        "0" => Code::Digit0, "1" => Code::Digit1, "2" => Code::Digit2, "3" => Code::Digit3,
        "4" => Code::Digit4, "5" => Code::Digit5, "6" => Code::Digit6, "7" => Code::Digit7,
        "8" => Code::Digit8, "9" => Code::Digit9,
        "F1" => Code::F1, "F2" => Code::F2, "F3" => Code::F3, "F4" => Code::F4,
        "F5" => Code::F5, "F6" => Code::F6, "F7" => Code::F7, "F8" => Code::F8,
        "F9" => Code::F9, "F10" => Code::F10, "F11" => Code::F11, "F12" => Code::F12,
        "Up" => Code::ArrowUp, "Down" => Code::ArrowDown,
        "Left" => Code::ArrowLeft, "Right" => Code::ArrowRight,
        "Space" => Code::Space, "Enter" => Code::Enter, "Escape" => Code::Escape,
        "Tab" => Code::Tab, "Backspace" => Code::Backspace, "Delete" => Code::Delete,
        "Comma" => Code::Comma, "Period" => Code::Period, "Slash" => Code::Slash,
        "Backslash" => Code::Backslash, "Minus" => Code::Minus, "Equal" => Code::Equal,
        "Semicolon" => Code::Semicolon, "Quote" => Code::Quote, "Backquote" => Code::Backquote,
        "BracketLeft" => Code::BracketLeft, "BracketRight" => Code::BracketRight,
        _ => return None,
    })
}

/// Tolerant X error handler: a failed grab must not abort the process.
fn install_tolerant_x_error_handler() {
    use std::ffi::c_int;
    unsafe extern "C" fn quiet(
        _d: *mut x11_dl::xlib::Display,
        _e: *mut x11_dl::xlib::XErrorEvent,
    ) -> c_int {
        0
    }
    if std::env::var_os("DISPLAY").is_none() {
        return;
    }
    if let Ok(xlib) = x11_dl::xlib::Xlib::open() {
        unsafe {
            (xlib.XSetErrorHandler)(Some(quiet));
        }
    }
}

/// Spawn the single hotkey-owning thread. `accel_rx` accepts new accelerators
/// (each replaces the previous registration).
pub fn spawn(events: EventLoopProxy<AppEvent>, accel_rx: Receiver<String>, initial: String) {
    std::thread::spawn(move || {
        install_tolerant_x_error_handler();
        let Ok(manager) = GlobalHotKeyManager::new() else { return };
        let register = |manager: &GlobalHotKeyManager, accel: &str| -> Option<HotKey> {
            let hk = parse_hotkey(accel)?;
            if manager.register(hk).is_ok() {
                Some(hk)
            } else {
                None
            }
        };
        let mut current = register(&manager, &initial);

        let key_rx = GlobalHotKeyEvent::receiver();
        loop {
            match accel_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(accel) => {
                    if let Some(old) = current.take() {
                        manager.unregister(old).ok();
                    }
                    current = register(&manager, &accel);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
            while let Ok(_ev) = key_rx.try_recv() {
                events.send_event(AppEvent::Toggle).ok();
            }
        }
    });
}
