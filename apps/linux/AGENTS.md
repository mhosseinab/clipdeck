# Linux implementation

Keep deterministic domain behavior in `crates/core`; keep clipboard, rendering,
tray, hotkey, filesystem environment wiring and event-loop integration in
`crates/app`. Keep domain behavior aligned with the contract and fixture oracle.

The app's current pattern has `main.rs` as the composition root, `AppState` on
the main thread, and cross-thread signals as `AppEvent` dispatched by `ui`.
Inspect `events.rs`, `state.rs`, `ui.rs` and the relevant producer before
changing that wiring. Avoid blocking the UI thread on clipboard or process I/O.

Read `Cargo.toml` and `Cargo.lock` for actual dependency versions. Keep existing
module and serde conventions. Add regression cases to
`crates/core/tests/fixtures.rs` and shared fixtures when behavior is shared.

From the repository root, run `bash scripts/check.sh linux-core` for core-only
changes or `bash scripts/check.sh linux` for app changes. The latter mirrors CI
tests and release compilation; it does not launch the daemon. A successful build
does not prove tray, shortcuts, X11/Wayland, focus or paste behavior.

For explicitly requested daemon testing, use synthetic data and set
`CLIPDECK_USER_DATA` to a temporary directory for both daemon and CLI commands.
Each canonical profile owns its private `.ipc/control.sock`; no global socket is
used. Never stop or replace a user's normal daemon without authorization.
