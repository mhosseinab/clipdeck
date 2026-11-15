# ClipDeck Behavior Contract

This document is normative. `MUST`, `SHOULD`, and `MAY` follow RFC 2119.
The JSON files under `fixtures/` are machine-readable test vectors for this
contract; where prose and fixtures disagree, the fixtures win.

Supported implementation: `apps/linux` (Rust). Linux is the only release target.
Behavior described here is reimplemented from observed feature evidence of a
commercial clipboard manager; no code or assets of that product are used.

---

## 1. Domain model

### 1.1 ClipItem

| Field | Type | Notes |
| --- | --- | --- |
| `id` | string | Unique per store. UUIDv4 recommended. |
| `type` | enum | `text`, `link`, `code`, `color`, `image`, `file`, `html` |
| `content` | string | Full payload. For `image`: the stored image file name. For `file`: newline-separated absolute paths. |
| `preview` | string | One-line derived summary (§4). |
| `title` | string | Derived short title (§4). |
| `sourceApp` | string \| null | Frontmost app name at capture time. |
| `createdAt` | integer | Unix epoch milliseconds. |
| `sizeBytes` | integer | UTF-8 byte length of `content` (0 for `file`). |
| `pinned` | boolean | Survives history trims. |
| `pinboardId` | string \| null | Membership in a pinboard also survives trims. |
| `stackOrder` | integer \| null | 1-based position in the paste stack; `null` = not queued. |
| `imageFile` | string? | Present only for `image`; equals `content`. |

### 1.2 Pinboard

`id` (string, unique), `name` (string), `icon` (string, default `"star"`),
`position` (integer, insertion order).

### 1.3 Settings

Exact defaults live in `fixtures/settings-defaults.json` (single source of
truth). Unknown settings keys read from disk MUST be preserved on write
(forward compatibility). Missing keys MUST fall back to the defaults.

---

## 2. Capture pipeline

### 2.1 Change detection

1. Poll the system clipboard every **400 ms** (±20% jitter allowed).
2. Compute `signature = formats ∪ "|" ∪ hash(text)` where `formats` is the
   sorted set of advertised format identifiers and `hash` is SHA-256 over the
   UTF-8 encoding of the plain-text reading.
3. If an image format is advertised, additionally hash the decoded image's
   encoded bytes (PNG) at most once every **2000 ms** per signature change;
   an image-hash change counts as a change.
4. The very first observation after start is recorded as the baseline and
   MUST NOT produce an item (nothing "new" was copied).
5. On change, run the capture rules below in order; exactly one item is
   produced per change.

### 2.2 Skip rules (evaluated in order)

1. **Watching disabled** (`settings.watchEnabled == false`): no capture.
2. **Transient**: if `settings.ignoreTransient` and the advertised formats
   contain any marker in `fixtures/transient-rules.json.markers`
   (e.g. `org.nspasteboard.TransientType`), skip the change entirely and
   update the baseline signature.
3. **Source app blocklist**: if `sourceApp` (§2.5) matches any entry of
   `settings.excludedApps` by case-insensitive substring, skip.
   Empty entries MUST be ignored.
4. **Confidential source**: if `settings.ignoreConfidential` and `sourceApp`
   matches the case-insensitive regex in
   `fixtures/transient-rules.json.confidentialAppPattern`, skip.

### 2.3 Content selection (first match wins)

1. **Files**: if file paths are advertised on the clipboard, produce one
   `file` item whose `content` is the newline-joined absolute paths.
2. **Image**: if an image payload exists and decodes non-empty, produce one
   `image` item; store the encoded PNG under the store's image directory as
   `<sha256-hex-of-png>.png`; `content` = that file name.
3. **Text**: read plain text `raw` and rich text `html` (if any). Let
   `body = raw` if non-empty, else `html` with tags stripped and whitespace
   collapsed. If `body.trim()` is empty, produce nothing.
   Type is `html` when rich text was present and §3 classification of `body`
   yields `text`; otherwise the §3 classification.

### 2.4 Deduplication

- Dedupe key: `type` + `"|"` + `content`, compared by exact string equality
  (no Unicode normalization; byte-identical UTF-8).
- Items with `stackOrder != null` are NEVER dedupe candidates.
- On duplicate: the existing item moves to the front of the list and its
  `createdAt` and `sourceApp` are refreshed. `pinned`, `pinboardId` keep
  their values. No new item is produced.

### 2.5 sourceApp

- Linux: window-manager title of the active window, or `null` when unknown.
- Capture is asynchronous by nature; `sourceApp` MAY be attributed up to
  250 ms after the change event.

### 2.6 Hook event `copy`

After an item is recorded (including dedupe-refresh), fire `copy` hooks (§8).

---

## 3. Text classification

Classifier (fixtures: `classify-text.json`):

```
classify(text):
  t = text.trimmed
  if t matches ^#[0-9a-fA-F]{3,8}$  -> color
  if t matches ^rgba?\(             -> color
  if t matches ^(https?:\/\/|www\.)\S+$ -> link
  score = 0
  if t matches [;{}()[\]]                                      -> score += 1
  if t matches (const |let |var |function |def |class |import |#include|<\/|\/>)  -> score += 1
  if t contains \n AND raw text matches / {2,}|\t/             -> score += 1
  return score >= 2 ? code : text
```

Notes (normative):
- The whitespace/indentation signal is tested against the **raw** (untrimmed)
  text; all other signals against the trimmed text.
- The indent regex is ` {2,}|\t` — two-or-more spaces OR one tab, anywhere.
- `html` is assigned only by §2.3, never by this classifier.

---

## 4. Preview and title derivation

All truncation counts **Unicode scalar values** (not UTF-16 units, not grapheme
clusters). Truncated strings end with the single scalar `U+2026` (`…`).

| type | title | preview |
| --- | --- | --- |
| `link` | hostname of the URL with leading `www.` removed; on parse failure: `content` truncated to 80 scalars | same as title |
| `color` | `content.trim()` | `content.trim()` |
| `image` | `Image` | `"<width>×<height> image"` (from decoded dimensions) |
| `file` | 1 file: base name; N files: `"<N> files"` | same as title |
| other | first line of `content`, trimmed, truncated to 60 scalars; empty first line yields the type name | first line of `content`, trimmed, truncated to 120 scalars |

Multi-line handling: "first line" = content up to the first `\n`.

---

## 5. Store format

Single JSON object, file name `clipdeck-store.json`, atomically replaced
(write temp + rename) with a 150 ms trailing debounce. Schema:
`schema/clipdeck-store.schema.json`. Example: `examples/sample-store.json`.

### 5.1 List order

`items[0]` is the most recent. Capture inserts at the front. `reorder(ids)`
reorders the whole list to match the given id order; ids not listed keep
their relative order after the listed ones.

### 5.2 Trim

```
keep     = items where pinned OR pinboardId != null OR stackOrder != null
overflow = all other items, in list order
budget   = max(0, settings.maxItems - |keep|)
kept     = filter items in original list order, retaining all keep items
           and only the first budget overflow items
```
- Protection affects retention only: pinned, board and stack entries MUST NOT
  move ahead of newer captures. Dedupe refresh still moves its entry to the front.
- `|keep|` may exceed `maxItems`; then no overflow item survives.
- Dropped `image` items have their image files deleted (best effort).

### 5.3 Import / export

- **Export**: JSON `{version: 1, exportedAt: <ISO-8601>, items, pinboards}`.
- **Import**: for each incoming item, skip when the dedupe key (§2.4) is
  present locally OR its `id` already exists locally (implementations MUST
  count skips). Malformed entries (missing `id`,
  non-string `content`) are skipped and counted. Imported items keep their
  `createdAt`; `stackOrder` is reset to `null`. Pinboards merge by `id`.
  Return `{imported, skipped}`.

---

## 6. Paste stack

- Push: `stackOrder = (count of items with stackOrder != null) + 1`.
- Pop: FIFO (`stackDirection: "fifo"`) takes the lowest `stackOrder`;
  LIFO (`"lifo"`) takes the highest. The popped item is copied to the
  clipboard, `paste` hooks fire, then the item is
  deleted from the store.
- `unstack(id)` sets `stackOrder = null`; `clearStack` unstacks all.
- Stack membership survives trim (§5.2).

---

## 7. Paste behavior

`pasteItem(id, plainText?)`:
1. Resolve `plain` = `plainText` if given (explicit), else
   `settings.pasteAsPlainText`, else true when the paste target app matches
   any `settings.plainTextApps` entry (case-insensitive substring).
2. Write to the system clipboard: `image` → PNG payload; `html` → rich text
   when `!plain` and the content contains markup; everything else, or any
   case with `plain == true`, → plain text.
3. Fire `paste` hooks (§8).
4. Hide the overlay. The user pastes into the destination application. Linux
   Enter/double-click activation copies without simulated keyboard or mouse
   input; Ctrl+C copies without hiding. See the Linux override in §13.

---

## 8. Scripting hooks

- Definition: `settings.hooks` entries `{event: "copy"|"paste", command}`.
  UI encoding `event=command` splits at the **first** `=`; both sides are
  trimmed; lines with unknown events or empty commands are dropped.
- Execution: `/bin/sh -c <command>` with the item's `content` written to the
  process's stdin (then closed). Timeout **5000 ms**, then kill.
- All matching hooks run concurrently. Hook failures are silent and MUST NOT
  affect the app.
- `copy` fires on capture and on explicit copy; `paste` fires on paste and
  stack-pop.

---

## 9. Sync (folder-based)

- Enabled when `settings.syncDirectory` is non-empty. File:
  `<dir>/clipdeck-sync.jsonl`, one JSON object per line:
  `{"v": 1, "item": <ClipItem>}` (schema: `schema/sync-line.schema.json`).
- **Append** every newly captured item (post-dedupe).
- **Merge** every **30 s** and at startup: parse lines added since the last
  merge (byte-offset tracked); skip malformed lines and lines whose dedupe
  key or `id` already exists locally; import the rest with `stackOrder`
  reset to `null`. After a merge that imported ≥ 1 item, the item list is
  sorted by `createdAt` descending (ties broken by `id` ascending). The
  return value is the imported count. Sync is best-effort: filesystem errors
  are silent.

---

## 10. Global shortcut

- Canonical accelerator grammar (fixtures: `accelerators.json`):

```
accelerator = modifiers? key
modifiers   = ("CommandOrControl" | "Command" | "Control")? "Alt"? "Shift"?
```
  `CommandOrControl` is recorded whenever either primary-modifier (Ctrl/Cmd)
  is pressed; remaining modifiers follow the fixed order above; `+` joins.
- Key vocabulary is derived from physical key codes:
  `KeyA..KeyZ` → `A..Z`, `Digit0..9` → `0..9`, `F1..F24` → `F1..F24`,
  `ArrowUp/Down/Left/Right` → `Up/Down/Left/Right`, and
  `Space, Enter, Escape, Tab, Backspace, Delete, Comma, Period, Slash,
  Backslash, Minus, Equal, Semicolon, Quote, Backquote, BracketLeft,
  BracketRight` map to themselves. Any other code is unsupported.
- A press of only modifier keys records nothing. `Escape` cancels recording.
- Registration: changing the accelerator unregisters the old one and
  registers the new one; on failure the old one is restored and the setting
  is NOT changed (result: `ok: false`).
- Display: `CommandOrControl→⌘/Ctrl`, `Command→⌘`, `Control→Ctrl`,
  `Shift→⇧`, `Alt→⌥`, joined with `+`.

---

## 11. Launch at login

Default: **enabled**.

- **Linux**: write `~/.config/autostart/clipdeck.desktop`, byte-exact content
  in `fixtures/autostart.json`; disable removes the file. Exec line:
  `Exec="<absolute-executable>"` (native binary needs no arguments; AppImage
  installs use the AppImage path).
- The setting is applied on change and re-applied at startup of packaged
  builds.

---

## 12. Privacy and UI flags

| Setting | Behavior |
| --- | --- |
| `ignoreTransient` | §2.2 rule 2 |
| `ignoreConfidential` | §2.2 rule 4 |
| `excludedApps` | §2.2 rule 3 |
| `generateLinkPreviews` | When true, `link` cards MAY fetch `<origin>/favicon.ico` (timeout 4000 ms, ≤ 512 KiB, cache per origin). When false, no network request is made. |
| `showInScreenShare` | When false, screen-capture exclusion is requested. Unsupported on the current Linux backend; do not rely on it to hide clipboard content. |
| `soundCopy` / `soundPaste` | Play short synthesized feedback on capture / paste. Sounds are original synthesized tones, bundled nowhere — generated at runtime. |

---

### Linux capability visibility

Transient-marker filtering, screen-share exclusion and network link previews
are unsupported in the current Linux capture/rendering backend. Settings MUST
label these controls as unsupported and omit interactive toggles. In a Wayland
session, clipboard-source identity is unknown: confidential-app filtering is
disabled and Settings explains that app exclusions cannot protect such captures.
Stored preference values remain preserved for format compatibility. Watch/pause
remains available. No OS permissions or simulated input are used to infer origins.

## 13. UI layer (informational)

The overlay presents: search field, sidebar (History / Pinned / pinboards),
horizontal card strip with per-type previews, paste-stack panel, hints row,
settings window. Linux keyboard: `←/→` select, `Enter` or double-click copy and
close, `Shift+Enter` copy plain and close, `Ctrl+E` edit, `Ctrl+C` copy without
closing, `Ctrl+S` stack, `Del` remove with confirmation, `Esc` close. Cards drag
to sidebar boards. Removing from a board preserves History unless the user
opts into deleting from History. Each card has a pin/unpin control. No synthetic
input is used. External drag-out and manual card reordering are not implemented.
Preview rendering shows at most 600 scalars of text/code.
