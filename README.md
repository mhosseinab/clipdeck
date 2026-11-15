# ClipDeck

Open-source clipboard manager — native apps for **macOS** (Swift) and
**Linux/Ubuntu** (Rust). A fast, searchable clipboard history with pinboards,
a paste stack, privacy rules, and a quick-paste overlay.

> **v2 native rewrite in progress.** The behavioral contract and cross-platform
> test fixtures live in [`spec/`](spec/) — both native implementations must
> pass them. The previous Electron implementation is frozen on the
> [`v1-electron`](https://github.com/mhosseinab/clipdeck/tree/v1-electron)
> branch; its releases remain downloadable.

## Layout

```
spec/          behavior contract (behavior.md) + shared test fixtures
  fixtures/    machine-readable vectors — run `node spec/verify.mjs`
  schema/      JSON Schemas for the store and sync formats
  examples/    golden example files
apps/
  macos/       Swift (AppKit + SwiftUI overlay)
  linux/       Rust workspace (core + app)
```

## Install (Linux)

```bash
sudo dpkg -i clipdeck_2.0.0-1_amd64.deb     # binary, desktop entry, icon
clipdeck                                     # daemon + tray (starts hidden)
```

- Global shortcut: `Ctrl+Shift+V` (recorder in Settings) — show/hide the overlay
- Tray menu: open, settings, pause watching, recent items, quit
- Runs at login (autostart entry, toggle in Settings)
- CLI: `clipdeck toggle|open|settings|quit|export|import`

Build the package yourself:

```bash
cd apps/linux
cargo build --release
cargo install cargo-deb
cargo deb -p clipdeck-app        # -> target/debian/clipdeck_*.deb
```

## Development

```bash
node spec/verify.mjs                          # fixture oracle (198 checks)
cd apps/linux && cargo test -p clipdeck-core  # Rust parity suite
cargo build --release && ./target/release/clipdeck
```

macOS: `cd apps/macos && swift test && swift build -c release`

## Format compatibility

The store (`clipdeck-store.json`) and sync (`clipdeck-sync.jsonl`) formats are
compatible with the v1 Electron app — existing history keeps working. Deviations
from v1 behavior are listed in `spec/behavior.md` §14.

## License

MIT
