//! Global shortcut (spec/behavior.md §10). `global-hotkey` covers X11; on
//! Wayland compositors without a global-shortcut protocol the registration
//! fails and the CLI (`clipdeck toggle`) or a DE shortcut binding is the path.

use crate::ui::Command;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager};
use std::sync::Arc;

/// Parse our canonical accelerator (e.g. "CommandOrControl+Shift+V") into a
/// global-hotkey HotKey. `CommandOrControl` maps to Ctrl on Linux.
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

pub fn parse_key_event_code(code: &str) -> Option<Code> {
    code_from_key(code)
}

/// x11-dl's default Xlib error handler aborts the process on BadAccess
/// (e.g. the accelerator is already grabbed). Install a tolerant handler so
/// a failed grab degrades gracefully; `clipdeck toggle` remains available.
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

/// Register the accelerator from settings and stream presses as `Command::Toggle`.
pub fn register(accel: &str, proxy: winit::event_loop::EventLoopProxy<Command>) -> Option<GlobalHotKeyManager> {
    install_tolerant_x_error_handler();
    let manager = GlobalHotKeyManager::new().ok()?;
    let hotkey = parse_hotkey(accel)?;
    if manager.register(hotkey).is_err() {
        return None;
    }
    std::thread::spawn(move || {
        let rx = GlobalHotKeyEvent::receiver();
        loop {
            match rx.recv() {
                Ok(_) => {
                    if proxy.send_event(Command::Toggle).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    Some(manager)
}

/// Re-register on settings change: unregister the old accelerator first (§10).
pub fn reregister(
    manager: &GlobalHotKeyManager,
    old_accel: &str,
    new_accel: &str,
) -> bool {
    if let Some(old) = parse_hotkey(old_accel) {
        manager.unregister(old).ok();
    }
    match parse_hotkey(new_accel) {
        Some(new) => manager.register(new).is_ok(),
        None => false,
    }
}

pub fn _unused(_: &Arc<()>) {}
