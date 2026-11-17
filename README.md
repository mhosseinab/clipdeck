# ClipDeck

Clipboard history for Linux. Find something you copied, organize it into
pinboards, and copy it again when you need it. Text, links, images, editable
clips, and a paste stack—all in a compact native overlay.

![ClipDeck clipboard history](assets/screenshots/clipdeck-history.png)

## Install

[![Get it from the Snap Store](https://snapcraft.io/en/light/install.svg)](https://snapcraft.io/clipdeck)

```bash
sudo snap install clipdeck
snap run clipdeck
```

To use **encrypted history**, connect ClipDeck to your desktop keyring before
choosing encryption:

```bash
sudo snap connect clipdeck:password-manager-service
```

This connection is optional for plaintext history.

The Snap supports **amd64 (x86_64) and arm64 (AArch64)**. Debian packages for both
are available from [GitHub Releases](https://github.com/mhosseinab/clipdeck/releases/latest).
See the [installation guide](docs/usage.md#install) for setup and encryption.

### Cargo

Install [clipdeck-app from crates.io](https://crates.io/crates/clipdeck-app):

```bash
cargo install clipdeck-app --locked
clipdeck run
```

This builds from source and installs the `clipdeck` executable.
See the [Linux build prerequisites](docs/development.md#build-and-run).

## Get started

1. **Copy** text, a link, or an image. ClipDeck keeps it while watching is enabled.
2. **Find** it in the overlay using search, pinned clips, or a pinboard.
3. **Reuse** it with Enter or a double-click, then paste into your application.

**Shift+Enter** copies plain text. **Escape** hides the overlay without quitting.
Use Settings or the tray menu to pause watching, change encryption, or control
launch at login.

On **X11**, open the overlay with **Ctrl+Shift+V**.

On **GNOME Wayland**, open system **Settings → Keyboard → View and Customize
Shortcuts → Custom Shortcuts**. Add **ClipDeck**, set the command to
`/snap/bin/clipdeck toggle`, and assign **Ctrl+Shift+V**. ClipDeck must already
be running. Open system Settings directly if the app's button fails.
For Cargo installations, use `/home/YOUR_USER/.cargo/bin/clipdeck toggle`
instead, replacing `YOUR_USER` with your username.

Encrypted history is optional and uses your desktop keyring. Exports and the
system clipboard stay plaintext. Pause watching when handling sensitive content.
[Privacy and backups →](docs/usage.md#privacy-and-backups)

## Documentation

- [User guide](docs/usage.md) — installation, shortcuts, CLI, and backups.
- [Development](docs/development.md) — build, test, and package from source.
- [Report a problem](https://github.com/mhosseinab/clipdeck/issues).

## License

[MIT](LICENSE)
