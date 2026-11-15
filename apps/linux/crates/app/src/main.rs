//! ClipDeck daemon for Linux: clipboard watcher, overlay UI, global shortcut,
//! CLI control channel. Behavior is defined by spec/behavior.md.

mod state;
mod tray;
mod watcher;
mod paste;
mod hotkey;
mod sounds;
mod ui;

use state::AppState;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const SOCKET_PATH: &str = "/tmp/clipdeck.sock";

fn user_data() -> PathBuf {
    if let Some(p) = std::env::var_os("CLIPDECK_USER_DATA") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join(".local").join("share").join("clipdeck")
}

fn exe_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("run") => run_daemon(),
        Some("toggle") | Some("open") | Some("settings") | Some("quit") => {
            send_command(args[0].clone());
        }
        Some("export") => {
            let state = Arc::new(Mutex::new(AppState::open(&user_data())));
            let path = args.get(1).cloned().unwrap_or_else(|| "clipdeck-export.json".into());
            let json = state.lock().unwrap().store.export_json();
            std::fs::write(&path, json).ok();
            println!("exported to {path}");
        }
        Some("import") => {
            let state = Arc::new(Mutex::new(AppState::open(&user_data())));
            if let Some(path) = args.get(1) {
                if let Ok(raw) = std::fs::read_to_string(path) {
                    let counts = state.lock().unwrap().store.import_json(&raw);
                    state.lock().unwrap().store.save();
                    println!("imported {}, skipped {}", counts.imported, counts.skipped);
                }
            }
        }
        Some("help") | Some("--help") | Some("-h") => print_help(),
        Some(other) => {
            eprintln!("unknown command: {other}");
            print_help();
        }
    }
}

fn print_help() {
    println!(
        "clipdeck — clipboard manager\n\n\
         Usage:\n\
         \x20 clipdeck [run]        start the daemon + overlay\n\
         \x20 clipdeck toggle      show/hide the overlay\n\
         \x20 clipdeck open        show the overlay\n\
         \x20 clipdeck settings    open settings\n\
         \x20 clipdeck quit        stop the daemon\n\
         \x20 clipdeck export [f]  export history JSON\n\
         \x20 clipdeck import [f]  import history JSON"
    );
}

/* ---------- single-instance control channel ---------- */

fn send_command(cmd: String) {
    match UnixStream::connect(SOCKET_PATH) {
        Ok(mut s) => {
            s.write_all(cmd.as_bytes()).ok();
        }
        Err(_) => eprintln!("clipdeck daemon is not running"),
    }
}

fn spawn_control_server(shared: Arc<Mutex<AppState>>, proxy: winit::event_loop::EventLoopProxy<ui::Command>) {
    let _ = std::fs::remove_file(SOCKET_PATH);
    if let Ok(listener) = UnixListener::bind(SOCKET_PATH) {
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut buf = String::new();
                s.read_to_string(&mut buf).ok();
                let cmd = match buf.trim() {
                    "quit" => ui::Command::Quit,
                    "settings" => ui::Command::OpenSettings,
                    "open" => ui::Command::Show,
                    _ => ui::Command::Toggle,
                };
                if proxy.send_event(cmd).is_err() {
                    break;
                }
                if buf.trim() == "quit" {
                    break;
                }
            }
        });
    }
}

/* ---------- daemon ---------- */

fn run_daemon() {
    let data = user_data();
    std::fs::create_dir_all(&data).ok();
    let shared = Arc::new(Mutex::new(AppState::open(&data)));

    // Launch-at-login per spec §11 (re-applied at startup of the daemon).
    {
        let st = shared.lock().unwrap();
        if st.store.settings.launch_at_login {
            clipdeck_core::apply_login_item_linux(true, &exe_path(), &dirs_home()).ok();
        }
    }

    let mut builder = winit::event_loop::EventLoop::<ui::Command>::with_user_event();
    // QA/test hook: force the X11 backend (XWayland) so input injection tools
    // can see the window; production defaults to Wayland-first selection.
    if std::env::var_os("CLIPDECK_FORCE_X11").is_some() {
        use winit::platform::x11::EventLoopBuilderExtX11;
        builder.with_x11();
    }
    let event_loop = builder.build().map_err(|e| e.to_string()).unwrap_or_else(|e| {
        eprintln!("event loop: {e}");
        std::process::exit(1);
    });
    let proxy = event_loop.create_proxy();
    spawn_control_server(shared.clone(), proxy.clone());
    tray::spawn(shared.clone(), proxy.clone());

    // Clipboard watcher thread (§2).
    let watch_state = shared.clone();
    let watch_proxy = proxy.clone();
    std::thread::spawn(move || watcher::run(watch_state, watch_proxy));

    // Sync merge ticker (§9).
    let sync_state = shared.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(30));
        let mut guard = sync_state.lock().unwrap();
        let st = &mut *guard;
        let counts = st.sync.merge(&mut st.store);
        if counts.imported > 0 {
            st.store.save();
            drop(st);
            // UI refresh happens on next paint tick via generation counter
        }
    });

    // Global shortcut (§10); on Wayland this may fail — CLI/DE binding works.
    let accel = shared.lock().unwrap().store.settings.shortcut.clone();
    hotkey::register(&accel, proxy);

    if let Err(e) = ui::run(shared, event_loop) {
        eprintln!("ui error: {e}");
    }
    let _ = std::fs::remove_file(SOCKET_PATH);
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}
