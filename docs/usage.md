# User guide

Installation options, keyboard shortcuts, CLI commands, privacy, and backups.
For a quick start, see the [README](../README.md).

## Install

### Snap Store — Ubuntu and other Snap-enabled distributions

The stable Snap is available for **amd64 (x86_64)** and **arm64 (AArch64)**.

```bash
sudo snap install clipdeck
snap run clipdeck
```

[View ClipDeck on the Snap Store](https://snapcraft.io/clipdeck).

For encrypted history, connect the desktop keyring before choosing encryption:

```bash
sudo snap connect clipdeck:password-manager-service
```

On first launch, choose encrypted or plaintext history. Later starts keep the
daemon hidden; open it from the tray or with `snap run clipdeck open`.

### Debian package — Ubuntu 24.04 or newer

Download the **amd64** or **arm64** `.deb` matching your machine from
[GitHub Releases](https://github.com/mhosseinab/clipdeck/releases/latest), then:

```bash
sudo apt install ./clipdeck_2.1.0-1_$(dpkg --print-architecture).deb
clipdeck
```

Encrypted history requires a Secret Service keyring and `libsecret-tools`.
Plaintext history works without them. If you install both the Snap and Debian
package, use `snap run clipdeck` to target the Snap explicitly; their profiles
are separate.

## Everyday use

1. Copy text, links, or images as usual. ClipDeck saves them while watching is on.
2. Open the overlay from the tray or your keyboard shortcut, then search or select a clip.
3. Press **Enter** or **double-click** to copy it. Paste into your destination app normally.

**Shift+Enter** copies plain text. **Escape** closes the overlay; in Settings,
it first returns to your clips. Closing the overlay keeps ClipDeck running.
Use the tray to pause watching or quit, and Settings to control launch at login.
GNOME may need a tray extension; the CLI also works without a tray.

Pin frequently used clips with the card's pin control. Create named pinboards
in the sidebar, then drag cards onto them or use a card's **↪** button. The pencil
renames a board. Deleting a board keeps its clips in History. Clip deletion
requires confirmation. Text clips can be edited, and the paste stack queues
clips for reuse.

### Keyboard shortcut

On **X11**, the default shortcut is **Ctrl+Shift+V**; change it in Settings.

On **GNOME Wayland**, add a custom shortcut in **Settings → Keyboard → View and
Customize Shortcuts → Custom Shortcuts**. For the Snap, use:

```text
/snap/bin/clipdeck toggle
```

For the Debian package, use `clipdeck toggle`. ClipDeck must already be running.
Settings → Global shortcut → **Copy Command** supplies the right command for
your installation. Choose keys that do not conflict with another application.

For a copy-paste command that registers Ctrl+Shift+V without replacing existing
GNOME shortcuts, expand the setup section in the [README](../README.md#get-started).
Run it from a normal desktop terminal. The Snap's **Keyboard Settings** button
may fail to open the host application; open GNOME Settings directly instead.

### CLI

For the Snap, use `snap run clipdeck` followed by a command:

| Command | Action |
| --- | --- |
| `open` | Show the overlay |
| `toggle` | Show or hide it |
| `settings` | Open Settings |
| `quit` | Stop the daemon |
| `export <file>` | Export history to JSON; quit first |
| `import <file>` | Import history from JSON; quit first |
| `--version` | Print the installed version |

For the Debian package, use `clipdeck` followed by the same commands.

## Privacy and backups

Encrypted history uses SQLCipher and a key stored in your desktop keyring;
image files are encrypted too. Losing the key makes that history unreadable.
You can change encryption mode in Settings; conversion requires a restart and
preserves existing SQLite history and images.

The system clipboard and JSON exports are **plaintext**, even with encrypted
history. Source-app identity may be unavailable on Wayland, so app exclusions
and confidential-content filters cannot reliably protect those captures.
Transient markers and screen-share exclusion are unsupported. **Pause watching
or quit ClipDeck when handling sensitive content.** Folder sync is disabled for
encrypted profiles.

To export the Snap's history:

```bash
snap run clipdeck quit
snap run clipdeck export "$HOME/clipdeck-export.json"
snap run clipdeck
```

JSON exports include clips and boards, but **not image-file bytes, settings, or
encryption keys**. For a full backup, quit ClipDeck and copy its entire profile:

- **Snap:** `~/snap/clipdeck/common/profile/`
- **Debian:** `$XDG_DATA_HOME/clipdeck/`, normally `~/.local/share/clipdeck/`

Encrypted backups also need the desktop-keyring key associated with the original
profile path. Moving the profile does not move that keyring association.
Before upgrading, back up the full profile. Legacy JSON history is not migrated;
existing SQLite history is preserved.
