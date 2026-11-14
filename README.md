# ClipDeck

Open-source clipboard manager for **macOS** and **Linux (Ubuntu)**, inspired by the
feature set of commercial clipboard managers — an original implementation with its
own code, name, and assets.

Copy anything and it lands instantly in a fast, searchable history strip: text,
links, code, colors, images, files and rich text. Paste it back anywhere with a
keystroke, organize into pinboards, and queue multiple items with the paste stack.

## Features

- **Clipboard watcher** — text, rich text, images, files, links, code, color values
- **Quick-paste overlay** — glass bottom sheet with a horizontal card strip (global shortcut, default `Ctrl/Cmd+Shift+V`)
- **Search** — instant filtering across full history (`Ctrl/Cmd+F`)
- **Pinboards** — pin items and organize into named boards via the sidebar
- **Paste stack** — queue several items (`Ctrl/Cmd+S`), paste them in order (FIFO/LIFO)
- **Paste as plain text** — `Shift+Enter`, or make it the default in Settings
- **Privacy rules** — honors clipboard *transient* marks (password managers), app blocklist, confidential-source skip
- **Sounds** — subtle synthesized copy/paste feedback (no sampled assets)
- **Tray icon** — pause watching, clear history, settings, quit
- **Settings** — history limits, shortcut, content rules, excluded apps

## Install & run

```bash
npm install
npm run dev        # vite + electron (development)
npm start          # production build + run
```

Linux note: development runs Electron with `--no-sandbox` (see `package.json`).
For the full Chromium sandbox instead:

```bash
sudo chown root:root node_modules/electron/dist/chrome-sandbox
sudo chmod 4755 node_modules/electron/dist/chrome-sandbox
```

## Keyboard

| Key | Action |
| --- | --- |
| `←` `→` | navigate cards |
| `Enter` | paste |
| `Shift+Enter` | paste as plain text |
| `Ctrl/Cmd+C` | copy |
| `Ctrl/Cmd+S` | push to paste stack |
| `Delete` | delete item |
| `Esc` | close overlay |

## Platform support

| Platform | Status |
| --- | --- |
| Linux (Wayland/X11) | working (`wl-clipboard`/`xclip`, `wtype`/`xdotool`) |
| macOS | compiles; runtime QA pending (osascript paste simulation) |

## Architecture

- Electron 33 + React 18 + TypeScript + Vite
- `electron/` — main process: clipboard watcher, store, tray, shortcuts, platform glue
- `src/` — renderer: overlay UI, settings, synthesized sounds
- History stored in a JSON file under the app's user-data directory (SQLite migration planned)

## License

MIT
