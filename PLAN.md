# ClipDeck — open-source clipboard manager for macOS + Linux

Goal: reimplement Paste.app 6.3.11 (com.wiheads.paste) feature set, layout and
interaction design as an original open-source app for macOS and Ubuntu.

## IP policy (fixed)

- Feature behavior, layout structure and interaction design are reimplemented from
  scratch as original code. All shipped assets (icons, sounds, copy text) original.
- NO copying of Paste's binaries, code, name, icons, sounds, artwork, or marketing copy.
- Product name: ClipDeck. No "Paste" trademark use.

## Evidence-based feature inventory

Source: string inventory of `paste-app/thin-Paste-x86_64` (kuna 1.833, 54,763 rows),
Ghidra 12.1.4 analysis, bundle Info.plist. Evidence rows are target *data*.

| # | Evidence (target strings/classes) | ClipDeck feature | Tier |
|---|---|---|---|
| 1 | "Clipboard history", "Unlimited history…", `ReduceHistoryLimit` | History w/ configurable cap | T0 done |
| 2 | `NSPasteboardWriting`, `public.utf8-plain-text`, `public.utf16-plain-text`, `public.file-url`, `public.png`, `public.text`, `public.rtf` | Multi-type capture (text/rtf/html/image/file/link/code/color) | T0 done |
| 3 | `MainWindowController`, `FilterBrowser*`, `PreviewPopover`, "Quick Pa…" | Quick-paste overlay + search | T0 done |
| 4 | `StatusItem`, `SidebarView`, `SidebarItemView` | Tray + pinboard sidebar | T0 rail pending |
| 5 | "Sound ef…" | Copy/paste sounds (synthesized originals) | T0 done |
| 6 | `com.wiheads.paste.item-id`, `items:pin`, "Pin items to pinboards…drag-and-drop" | Pinboards, pin/drag | T0 pin done; drag T1 |
| 7 | `org.nspasteboard.TransientType`, `de.petermaurer.TransientPasteboardType`, "Ignore transient content" | Honor transient clipboard marks (never store) | T1 |
| 8 | "Ignore confidential content", `ConfidentialContent`, `com.apple.keychainaccess`, `com.apple.Passwords` | Sensitive-source rules | T1 |
| 9 | "Ignore Applications" | App blocklist (by source app) | T1 |
| 10 | "Always paste as Plain Text" | Paste-as-plain-text (per paste + default) | T1 |
| 11 | `EnableDirectPaste`, `DirectPasteAction`, "Do you want to paste directly to %@?" | Direct paste into target app | T0 paste done; direct-to-app picker T2 |
| 12 | `PasteStackWindow*`, `PasteStackTableView`, `paste-stack-shortcut`, `paste-stack-direction`, "Activate Paste Stack" | Paste stack (ordered multi-paste, direction) | T1 |
| 13 | "Generate link previews" | Link preview cards (toggle) | T1 |
| 14 | `EditorHeader`, `ColorPickerCell`, `ItemsSharingPicker` | Item editor / color preview / share | T1/T2 |
| 15 | "Activation shortcut", "Reset shortcuts to default" | Configurable shortcuts | T0 done |
| 16 | `EnableAccessibilityAccess`, `EnablePasteboardAccess` | Permission guidance (macOS) | T2 |
| 17 | `EnableSync`/iCloud strings | Self-hosted optional sync (NOT iCloud) | T2 |
| 18 | `com.wiheads.paste.paste-stack-frame`, "Allow Paste to appear to others when you share your screen" | Overlay window sharing flag (macOS) | T2 |
| OUT | `PaywallViewController`, StoreKit subscriptions, Paste Keyboard (iOS), Widget, PostHog analytics | Out of scope (licensing/telemetry/other platforms) | — |

## Stack

- Electron 33 + React 18 + TypeScript + Vite; JSON-file store (SQLite in T2 if needed).
- Platform layer: mac (osascript/JXA), linux-x11 (xclip/xdotool), linux-wayland (wl-paste/wtype).
- Dev loop: `npm run dev` (vite + electron). Sandbox note: linux dev uses `--no-sandbox`
  or SUID chrome-sandbox; packaging handles it.

## Status

- [x] Providers installed & configured (Ghidra 12.1.4, Kuna 1.833, IDA Free 9.3)
- [x] Thin x86_64 slice extracted; strings/feature inventory mined (54,763 strings)
- [x] T0: watcher + store + overlay + card strip + search + copy/paste + tray + sounds + settings
- [x] T1 complete: transient/confidential rules, app blocklist, plain-text paste (+ per-app),
      paste stack panel (FIFO/LIFO), sidebar rail, favicon link previews, code syntax
      highlighting, item editor, drag reorder + drag-out, tray recents, screen-share flag
- [x] T2 complete: packaging (AppImage/deb/dmg/zip) + CI release workflow, export/import,
      folder-based sync, scripting hooks (copy/paste), auto-update scaffold (electron-updater)
- [x] Verification: 7/7 logic tests pass (store dedupe/trim/export-import, sync merge, hooks),
      tsc clean (src + electron), runtime smoke on Ubuntu (capture + new settings schema),
      `electron-builder` packaging passes
- [ ] macOS runtime QA (builds shipped unsigned via CI)
