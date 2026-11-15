//! ClipDeck daemon for Linux. Composition root only: CLI, thread wiring,
//! lifecycle. All state lives in `AppState` on the main thread; every
//! cross-thread signal travels as `AppEvent` through one dispatcher in `ui`.

mod draw;
mod events;
mod hotkey;
mod page_loader;
mod presentation;
mod sounds;
mod security;
mod state;
mod tray;
mod ui;
mod worker;

use events::AppEvent;
use state::AppState;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::time::Duration;
use std::sync::mpsc::channel;
use std::sync::{Arc, RwLock};

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");
pub(crate) const APP_ID: &str = "io.github.mhosseinab.clipdeck";

const MAX_CONTROL_BYTES: u64 = 32;

fn user_data() -> PathBuf {
    profile_directory(std::env::var_os("CLIPDECK_USER_DATA"),
        std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"))
}

fn profile_directory(override_path: Option<std::ffi::OsString>, xdg_data: Option<std::ffi::OsString>, home: Option<std::ffi::OsString>) -> PathBuf {
    if let Some(p) = override_path {
        return PathBuf::from(p);
    }
    if let Some(path) = xdg_data.map(PathBuf::from).filter(|path| path.is_absolute()) {
        return path.join("clipdeck");
    }
    let home = home.map(PathBuf::from).unwrap_or_default();
    home.join(".local").join("share").join("clipdeck")
}

fn exe_path() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("run") => run_daemon(),
        Some("toggle") | Some("open") | Some("settings") | Some("quit") => {
            if let Err(error) = send_command(&args[0]) {
                eprintln!("cannot contact ClipDeck for this profile: {error}");
                std::process::exit(1);
            }
        }
        Some("export") => {
            let path = args.get(1).map(String::as_str).unwrap_or("clipdeck-export.json");
            let data = user_data();
            let _profile_lock = lock_profile(&data).unwrap_or_else(|error| {
                eprintln!("cannot export while the profile is in use; quit ClipDeck first: {error}");
                std::process::exit(1);
            });
            if let Err(error) = export_profile(&data, Path::new(path)) {
                eprintln!("cannot export ClipDeck history: {error}");
                std::process::exit(1);
            }
            println!("exported to {path}");
        }
        Some("import") => {
            let Some(path) = args.get(1) else {
                eprintln!("usage: clipdeck import <file>");
                std::process::exit(1);
            };
            let data = user_data();
            let _profile_lock = lock_profile(&data).unwrap_or_else(|error| {
                eprintln!("cannot import while the profile is in use; quit ClipDeck first: {error}");
                std::process::exit(1);
            });
            match import_profile(&data, Path::new(path)) {
                Ok(counts) => println!("imported {}, skipped {}", counts.imported, counts.skipped),
                Err(error) => {
                    eprintln!("cannot import ClipDeck history: {error}");
                    std::process::exit(1);
                }
            }
        }
        Some("--version") | Some("-V") => println!("ClipDeck {VERSION}"),
        Some("help") | Some("--help") | Some("-h") => print_help(),
        Some(other) => {
            eprintln!("unknown command: {other}");
            print_help();
        }
    }
}

fn export_profile(data: &Path, path: &Path) -> Result<(), String> {
    let destination = export_destination(data, path).map_err(|error| error.to_string())?;
    let app = AppState::try_open(data)?;
    write_export_atomically(&destination, |output| app.store.export_to_writer(output))
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn export_destination(profile: &Path, path: &Path) -> std::io::Result<PathBuf> {
    let invalid = || std::io::Error::new(std::io::ErrorKind::InvalidInput,
        "export destination must not replace ClipDeck profile data or use a linked file");
    let profile = profile.canonicalize()?;
    let name = path.file_name().ok_or_else(invalid)?;
    let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new(".")).canonicalize()?;
    let destination = parent.join(name);
    if let Ok(relative) = destination.strip_prefix(&profile) {
        let first = relative.components().next().ok_or_else(invalid)?;
        let first = first.as_os_str().to_string_lossy();
        if matches!(first.as_ref(), "store" | ".ipc" | "images" | "instance.lock"
            | "clipdeck-security.json" | "clipdeck-store.json" | ".clipdeck-storage")
            || first.starts_with("store-") || first.starts_with("clipdeck.sqlite3")
            || first.starts_with(".security-") || first.starts_with(".export-") {
            return Err(invalid());
        }
    }
    match std::fs::symlink_metadata(&destination) {
        Ok(metadata) if !metadata.is_file() || metadata.nlink() != 1 => return Err(invalid()),
        Ok(_) => {},
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(error) => return Err(error),
    }
    Ok(destination)
}

fn write_export_atomically(
    destination: &Path,
    write: impl FnOnce(&mut std::io::BufWriter<std::fs::File>) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let parent = destination.parent().expect("validated export parent");
    let temporary = parent.join(format!(".export-{}.tmp", clipdeck_core::store::uuid_v4ish()));
    let file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600)
        .open(&temporary)?;
    let result = (|| {
        let mut output = std::io::BufWriter::new(file);
        write(&mut output)?;
        output.flush()?;
        output.get_ref().sync_all()?;
        std::fs::rename(&temporary, destination)?;
        std::fs::File::open(parent)?.sync_all()
    })();
    let _ = std::fs::remove_file(temporary);
    result
}

fn import_profile(data: &Path, path: &Path) -> Result<clipdeck_core::ImportCounts, String> {
    let file = std::fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut app = AppState::try_open(data)?;
    let counts = app.store.import_from_reader(std::io::BufReader::new(file))
        .map_err(|error| error.to_string())?;
    app.save();
    Ok(counts)
}

fn print_help() {
    println!(
        "clipdeck {VERSION} — clipboard manager\n\n\
         Usage:\n\
         \x20 clipdeck --version   print the application version\n\
         \x20 clipdeck [run]        start the daemon + overlay\n\
         \x20 clipdeck toggle      show/hide the overlay\n\
         \x20 clipdeck open        show the overlay\n\
         \x20 clipdeck settings    open settings\n\
         \x20 clipdeck quit        stop the daemon\n\
         \x20 clipdeck export [f]  export history JSON\n\
         \x20 clipdeck import [f]  import history JSON"
    );
}

/* ---------- single-instance control channel -> AppEvents ---------- */

fn send_command(cmd: &str) -> std::io::Result<()> {
    let path = control_socket_path(&user_data())?;
    verify_private_directory(path.parent().expect("control socket parent"))?;
    let mut stream = UnixStream::connect(path)?;
    stream.set_write_timeout(Some(Duration::from_millis(250)))?;
    stream.write_all(cmd.as_bytes())
}

fn current_uid() -> std::io::Result<u32> {
    // Linux procfs reports the owner of this process without an external command
    // or trusting a user-provided UID environment variable.
    Ok(std::fs::metadata("/proc/self")?.uid())
}

fn owned_directory(path: &Path) -> std::io::Result<std::fs::Metadata> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.uid() != current_uid()? || metadata.mode() & 0o022 != 0 {
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied,
            format!("{} must be an owned directory, not writable by other users", path.display())));
    }
    Ok(metadata)
}

fn verify_private_directory(path: &Path) -> std::io::Result<()> {
    let metadata = owned_directory(path)?;
    if metadata.mode() & 0o077 != 0 {
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied,
            format!("{} must have owner-only permissions (0700)", path.display())));
    }
    Ok(())
}

fn control_socket_path(data: &Path) -> std::io::Result<PathBuf> {
    let profile = data.canonicalize()?;
    owned_directory(&profile)?;
    Ok(profile.join(".ipc").join("control.sock"))
}

fn prepare_control_socket(data: &Path) -> std::io::Result<PathBuf> {
    let socket = control_socket_path(data)?;
    let directory = socket.parent().expect("control socket parent");
    match std::fs::DirBuilder::new().mode(0o700).create(directory) {
        Ok(()) => {},
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {},
        Err(error) => return Err(error),
    }
    verify_private_directory(directory)?;
    Ok(socket)
}

fn lock_profile(data: &Path) -> std::io::Result<std::fs::File> {
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(data)?;
    let profile = data.canonicalize()?;
    let metadata = std::fs::symlink_metadata(&profile)?;
    if !metadata.is_dir() || metadata.uid() != current_uid()? {
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied,
            "profile must be a directory owned by the current user"));
    }
    // Earlier versions inherited umask (commonly 0002), allowing group writes.
    // Seal only this app-owned profile before creating its private endpoint.
    std::fs::set_permissions(&profile, std::fs::Permissions::from_mode(0o700))?;
    let path = profile.join("instance.lock");
    if let Ok(metadata) = std::fs::symlink_metadata(&path) {
        if !metadata.is_file() || metadata.uid() != current_uid()? {
            return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "invalid profile lock file"));
        }
    }
    let lock = std::fs::OpenOptions::new().create(true).truncate(false)
        .read(true).write(true).mode(0o600).open(path)?;
    lock.try_lock().map_err(std::io::Error::from)?;
    // Keep this descriptor alive for every operation that can write the store.
    // Never unlink the lock file: that would allow locking a different inode.
    Ok(lock)
}

/// Called while holding the profile lock, so stale recovery cannot race another
/// ClipDeck process for this profile. Never unlink a non-socket path.
fn bind_control_listener(path: &Path) -> std::io::Result<UnixListener> {
    let bind = || -> std::io::Result<UnixListener> {
        let listener = UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(listener)
    };
    match bind() {
        Ok(listener) => Ok(listener),
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            if UnixStream::connect(path).is_ok() {
                return Err(error);
            }
            let metadata = std::fs::symlink_metadata(path)?;
            if !metadata.file_type().is_socket() || metadata.uid() != current_uid()? {
                return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied,
                    "refusing to remove a non-socket or foreign control endpoint"));
            }
            std::fs::remove_file(path)?;
            bind()
        }
        Err(error) => Err(error),
    }
}

fn read_control_event(stream: &mut UnixStream) -> std::io::Result<Option<AppEvent>> {
    stream.set_read_timeout(Some(Duration::from_millis(250)))?;
    let mut command = String::new();
    stream.take(MAX_CONTROL_BYTES + 1).read_to_string(&mut command)?;
    if command.len() as u64 > MAX_CONTROL_BYTES {
        return Ok(None);
    }
    Ok(match command.trim() {
        "quit" => Some(AppEvent::Quit),
        "settings" => Some(AppEvent::OpenSettings),
        "open" => Some(AppEvent::Show),
        "toggle" => Some(AppEvent::Toggle),
        _ => None,
    })
}

fn spawn_control_server(listener: UnixListener, events: winit::event_loop::EventLoopProxy<AppEvent>) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let Ok(Some(event)) = read_control_event(&mut s) else { continue };
            let quit = matches!(event, AppEvent::Quit);
            if events.send_event(event).is_err() || quit {
                break;
            }
        }
    });
}

/* ---------- daemon ---------- */

fn lock_daemon_profile(data: &Path) -> std::io::Result<std::fs::File> {
    let restarting = std::env::var_os("CLIPDECK_ENCRYPTION_RESTART").is_some();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match lock_profile(data) {
            Err(error) if restarting && error.kind() == std::io::ErrorKind::WouldBlock && std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            result => return result,
        }
    }
}

fn run_daemon() {
    let data = user_data();
    let _profile_lock = match lock_daemon_profile(&data) {
        Ok(lock) => lock,
        Err(error) => {
            eprintln!("ClipDeck profile is already in use or unavailable: {error}");
            std::process::exit(1);
        }
    };
    // Each canonical profile has a private endpoint. CLI commands must use the
    // same CLIPDECK_USER_DATA value; there is no shared /tmp fallback.
    let socket = prepare_control_socket(&data).unwrap_or_else(|error| {
        eprintln!("cannot prepare ClipDeck control socket: {error}");
        std::process::exit(1);
    });
    let listener = bind_control_listener(&socket).unwrap_or_else(|error| {
        eprintln!("cannot bind ClipDeck control socket {}: {error} (Unix socket paths must fit the platform limit)", socket.display());
        std::process::exit(1);
    });
    let app = AppState::try_open_for_daemon(&data).unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(1);
    });

    // Launch-at-login per spec §11 (re-applied at startup of the daemon).
    if app.settings().launch_at_login {
        clipdeck_core::apply_login_item_linux(true, &exe_path(), &home()).ok();
    }

    let mut builder = winit::event_loop::EventLoop::<AppEvent>::with_user_event();
    let force_x11 = std::env::var("CLIPDECK_FORCE_X11").ok().map(|value| value != "0");
    if prefer_x11(force_x11, std::env::var_os("WAYLAND_DISPLAY").is_some(),
        std::env::var_os("DISPLAY").is_some(), &std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default()) {
        use winit::platform::x11::EventLoopBuilderExtX11;
        builder.with_x11();
    }
    let event_loop = builder.build().map_err(|e| e.to_string()).unwrap_or_else(|e| {
        eprintln!("event loop: {e}");
        std::process::exit(1);
    });
    let events = event_loop.create_proxy();

    // Producers: control socket, clipboard worker, hotkey, tray.
    spawn_control_server(listener, events.clone());

    let (worker_tx, worker_rx) = channel();
    worker::spawn(events.clone(), worker_rx, app.settings().watch_enabled && app.privacy.configured);

    let (accel_tx, accel_rx) = channel();
    let initial_accel = app.settings().shortcut.clone();
    hotkey::spawn(events.clone(), accel_rx, initial_accel);

    let tray_view = Arc::new(RwLock::new(tray::TrayView {
        watching: app.settings().watch_enabled && app.privacy.configured,
        recent: vec![],
    }));
    tray::spawn(tray_view.clone(), events.clone());

    if std::env::var_os("CLIPDECK_ENCRYPTION_RESTART").is_some() {
        let _ = events.send_event(AppEvent::Show);
    }
    if let Err(e) = ui::run(app, events, worker_tx, accel_tx, tray_view, event_loop) {
        eprintln!("ui error: {e}");
    }
    let _ = std::fs::remove_file(socket);
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

fn prefer_x11(force: Option<bool>, wayland: bool, display: bool, desktop: &str) -> bool {
    // GNOME's native Wayland protocol has no client always-on-top request.
    // Reuse the existing XWayland path, where the ABOVE hint is supported.
    force.unwrap_or_else(|| wayland && display && desktop.split(':').any(|part| part.eq_ignore_ascii_case("gnome")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_rejects_native_profile_paths_without_changing_them() {
        let profile = std::env::temp_dir().join(format!("clipdeck-export-{}", clipdeck_core::store::uuid_v4ish()));
        let _lock = lock_profile(&profile).unwrap();
        export_profile(&profile, &profile.join("export.json")).unwrap();
        let policy = profile.join("clipdeck-security.json");
        let before = std::fs::read(&policy).unwrap();
        assert!(export_profile(&profile, &policy).is_err());
        assert_eq!(std::fs::read(&policy).unwrap(), before);
        std::fs::remove_dir_all(profile).unwrap();
    }

    #[test]
    fn export_paths_resolve_aliases_and_protect_all_reserved_locations() {
        let profile = std::env::temp_dir().join(format!("clipdeck-export-paths-{}", clipdeck_core::store::uuid_v4ish()));
        let _lock = lock_profile(&profile).unwrap();
        for directory in ["store", "store-old", ".ipc", "images", "ordinary"] {
            std::fs::create_dir(profile.join(directory)).unwrap();
        }
        for path in ["store/clipdeck.sqlite3", "store-old/export.json", ".ipc/control.sock",
            "instance.lock", "clipdeck-security.json", "clipdeck.sqlite3-wal", "images/new.png",
            "clipdeck-store.json", ".security-test.tmp", ".clipdeck-storage"] {
            assert!(export_destination(&profile, &profile.join(path)).is_err(), "{path}");
        }
        let alias = profile.join("alias");
        std::os::unix::fs::symlink(profile.join("store"), &alias).unwrap();
        assert!(export_destination(&profile, &alias.join("new.json")).is_err());
        let link = profile.join("linked.json");
        std::os::unix::fs::symlink(profile.join("instance.lock"), &link).unwrap();
        assert!(export_destination(&profile, &link).is_err());
        let hardlink = profile.join("hardlinked.json");
        std::fs::hard_link(profile.join("instance.lock"), &hardlink).unwrap();
        assert!(export_destination(&profile, &hardlink).is_err());
        assert!(export_destination(&profile, &profile).is_err());
        assert!(export_destination(&profile, &profile.join("export.json")).is_ok());
        assert!(export_destination(&profile, &profile.join("ordinary/export.json")).is_ok());
        std::fs::remove_dir_all(profile).unwrap();
    }

    #[test]
    fn export_atomic_write_preserves_existing_output_on_error_and_sets_private_mode() {
        let directory = std::env::temp_dir().join(format!("clipdeck-export-atomic-{}", clipdeck_core::store::uuid_v4ish()));
        std::fs::create_dir(&directory).unwrap();
        let destination = directory.join("export.json");
        std::fs::write(&destination, b"previous export").unwrap();
        let result = write_export_atomically(&destination, |output| {
            output.write_all(b"partial plaintext")?;
            Err(std::io::Error::other("serialization failed"))
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"previous export");
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        write_export_atomically(&destination, |output| output.write_all(b"complete export")).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"complete export");
        assert_eq!(destination.metadata().unwrap().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn profile_directory_respects_xdg_and_explicit_override() {
        let home = Some("/home/test".into());
        let xdg = Some("/sandbox/data".into());
        assert_eq!(profile_directory(None, xdg.clone(), home.clone()), PathBuf::from("/sandbox/data/clipdeck"));
        assert_eq!(profile_directory(Some("/custom".into()), xdg, home), PathBuf::from("/custom"));
    }

    #[test]
    fn profile_directory_ignores_empty_or_relative_xdg_paths() {
        for xdg in [None, Some("".into()), Some("relative/data".into())] {
            assert_eq!(profile_directory(None, xdg, Some("/home/test".into())), PathBuf::from("/home/test/.local/share/clipdeck"));
        }
    }

    #[test]
    fn control_socket_is_private_and_profile_scoped() {
        let root = std::env::temp_dir().join(format!("clipdeck-ipc-{}", clipdeck_core::store::uuid_v4ish()));
        let a = root.join("a");
        let b = root.join("b");
        let _a_lock = lock_profile(&a).unwrap();
        let _b_lock = lock_profile(&b).unwrap();
        let a_socket = prepare_control_socket(&a).unwrap();
        let b_socket = prepare_control_socket(&b).unwrap();
        assert_ne!(a_socket, b_socket);
        assert_eq!(control_socket_path(&a).unwrap(), a_socket);
        assert_eq!(a_socket.parent().unwrap().metadata().unwrap().permissions().mode() & 0o777, 0o700);
        let a_listener = bind_control_listener(&a_socket).unwrap();
        let b_listener = bind_control_listener(&b_socket).unwrap();
        assert_eq!(a_socket.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        assert!(UnixStream::connect(&a_socket).is_ok());
        assert!(UnixStream::connect(&b_socket).is_ok());
        drop((a_listener, b_listener));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn private_ipc_rejects_symlink_and_open_permissions() {
        let root = std::env::temp_dir().join(format!("clipdeck-insecure-ipc-{}", clipdeck_core::store::uuid_v4ish()));
        let _lock = lock_profile(&root).unwrap();
        let outside = root.join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join(".ipc")).unwrap();
        assert!(prepare_control_socket(&root).is_err());
        std::fs::remove_file(root.join(".ipc")).unwrap();
        std::fs::create_dir(root.join(".ipc")).unwrap();
        std::fs::set_permissions(root.join(".ipc"), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(prepare_control_socket(&root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn control_input_is_bounded_and_idle_clients_time_out() {
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        writer.write_all(b"open").unwrap();
        writer.shutdown(std::net::Shutdown::Write).unwrap();
        assert!(matches!(read_control_event(&mut reader).unwrap(), Some(AppEvent::Show)));
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        writer.write_all(&[b'x'; 100]).unwrap();
        assert!(read_control_event(&mut reader).unwrap().is_none());
        let (mut reader, _idle_writer) = UnixStream::pair().unwrap();
        assert!(read_control_event(&mut reader).is_err());
    }

    #[test]
    fn profile_lock_prevents_two_store_writers_and_releases_on_drop() {
        let data = std::env::temp_dir().join(format!("clipdeck-lock-{}", clipdeck_core::store::uuid_v4ish()));
        std::fs::create_dir_all(&data).unwrap();
        let first = lock_profile(&data).unwrap();
        assert!(lock_profile(&data).is_err());
        drop(first);
        assert!(lock_profile(&data).is_ok());
        std::fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn control_listener_preserves_live_socket_and_recovers_stale_socket() {
        let path = std::env::temp_dir().join(format!("clipdeck-socket-{}", clipdeck_core::store::uuid_v4ish()));
        let first = bind_control_listener(&path).unwrap();
        assert!(bind_control_listener(&path).is_err());
        assert!(UnixStream::connect(&path).is_ok());
        drop(first);
        let recovered = bind_control_listener(&path).unwrap();
        drop(recovered);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn gnome_uses_xwayland_for_supported_always_on_top() {
        assert!(prefer_x11(None, true, true, "ubuntu:GNOME"));
        assert!(!prefer_x11(None, true, false, "GNOME"));
        assert!(!prefer_x11(Some(false), true, true, "GNOME"));
        assert!(prefer_x11(Some(true), false, true, ""));
    }
}
