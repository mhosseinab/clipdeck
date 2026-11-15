# ClipDeck

Clipboard history for Linux. Find something you copied, organize it into
pinboards, and copy it again when you need it. Text, links, images, editable
clips, and a paste stack—all in a compact native overlay.

![ClipDeck clipboard history](assets/screenshots/clipdeck-history.png)

## Install

<iframe src="https://snapcraft.io/clipdeck/embedded?button=white" frameborder="0" width="100%" height="450px" style="border: 1px solid #CCC; border-radius: 2px;"></iframe>

<!-- GitHub removes iframes; the official Store badge below remains usable. -->
[![Get it from the Snap Store](https://snapcraft.io/en/light/install.svg)](https://snapcraft.io/clipdeck)

```bash
sudo snap install clipdeck
snap run clipdeck
```

The Snap supports **amd64 (x86_64)**. Debian packages for **amd64 and arm64**
are available from [GitHub Releases](https://github.com/mhosseinab/clipdeck/releases/latest).
See the [installation guide](docs/usage.md#install) for setup and encryption.

## Get started

1. **Copy** text, a link, or an image. ClipDeck keeps it while watching is enabled.
2. **Find** it in the overlay using search, pinned clips, or a pinboard.
3. **Reuse** it with Enter or a double-click, then paste into your application.

**Shift+Enter** copies plain text. **Escape** hides the overlay without quitting.
Use Settings or the tray menu to pause watching, change encryption, or control
launch at login.

On **X11**, open the overlay with **Ctrl+Shift+V**. On **GNOME Wayland**, create a
custom keyboard shortcut with the command `/snap/bin/clipdeck toggle`.
ClipDeck must already be running. [Shortcut setup →](docs/usage.md#keyboard-shortcut)

Encrypted history is optional and uses your desktop keyring. Exports and the
system clipboard stay plaintext. Pause watching when handling sensitive content.
[Privacy and backups →](docs/usage.md#privacy-and-backups)

## Documentation

- [User guide](docs/usage.md) — installation, shortcuts, CLI, and backups.
- [Development](docs/development.md) — build, test, and package from source.
- [Report a problem](https://github.com/mhosseinab/clipdeck/issues).

## License

[MIT](LICENSE)
