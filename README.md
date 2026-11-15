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

<details>
<summary>Enable Ctrl+Shift+V for the Snap on GNOME Wayland</summary>

Run this in your normal desktop terminal. It adds the ClipDeck shortcut while
keeping your existing custom shortcuts:

```bash
python3 - <<'PY'
import ast
import subprocess

schema = "org.gnome.settings-daemon.plugins.media-keys"
path = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/clipdeck/"

def settings(*args):
    return subprocess.check_output(["gsettings", *args], text=True).strip()

bindings = ast.literal_eval(settings("get", schema, "custom-keybindings").removeprefix("@as "))
for key, value in {"name": "ClipDeck", "command": "/snap/bin/clipdeck toggle", "binding": "<Control><Shift>v"}.items():
    settings("set", f"{schema}.custom-keybinding:{path}", key, repr(value))
if path not in bindings:
    settings("set", schema, "custom-keybindings", repr([*bindings, path]))
print("ClipDeck shortcut enabled: Ctrl+Shift+V")
PY
```

The Snap's **Keyboard Settings** button may be unable to launch the host's
settings application. Use this command or open GNOME Settings directly.

</details>

Encrypted history is optional and uses your desktop keyring. Exports and the
system clipboard stay plaintext. Pause watching when handling sensitive content.
[Privacy and backups →](docs/usage.md#privacy-and-backups)

## Documentation

- [User guide](docs/usage.md) — installation, shortcuts, CLI, and backups.
- [Development](docs/development.md) — build, test, and package from source.
- [Report a problem](https://github.com/mhosseinab/clipdeck/issues).

## License

[MIT](LICENSE)
