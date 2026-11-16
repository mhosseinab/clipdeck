# Local Snap candidate

From the repository root, with Snapcraft and an initialized LXD build provider:

```sh
mkdir -p dist
snapcraft pack --platform amd64 --output dist/clipdeck_2.1.2_amd64.snap
sudo snap install --dangerous ./dist/clipdeck_2.1.2_amd64.snap
sudo snap connect clipdeck:password-manager-service
```

The last command grants access to the desktop Secret Service, required for
encrypted storage. It does not normally auto-connect. A locked or unavailable
keyring must remain an error; do not bypass it with plaintext storage.

The project declares native `amd64` and `arm64` platforms. For an ARM64 build
from an x86_64 machine, use Snapcraft's Launchpad builders:

```sh
snapcraft remote-build --build-for arm64
```

Use a full Git clone; shallow clones are not supported. Launchpad requires
Ubuntu One authorization and uploads the build source publicly. The build runs
the contract oracle and workspace tests natively before packaging. Downloaded
ARM64 artifacts need package inspection before upload or release. Native ARM
desktop behavior still requires a graphical ARM64 system to verify.

This is a strictly confined stable-grade candidate. Building it does not publish it.
The launcher uses `$SNAP_USER_COMMON/profile` for all commands, keeping native
installation history separate. There is no automatic migration of existing data.

Before starting the app, initialize a fresh test profile via
`snap run clipdeck export "$HOME/snap/clipdeck/common/qa-export.json"`, then
follow the isolated plaintext QA preparation in `AGENTS.md` using
synthetic records and disabling watching, hooks, sync and autostart. Use
`snap run clipdeck open` and `snap run clipdeck quit` to target this profile.
Do not run a second watcher against the user's live clipboard for automated QA.

Verify installation and refresh preserve the database and images; check CLI IPC,
first-launch privacy choice, encrypted mode, tray, text/image copying, X11 and
Wayland separately. Capture frame dumps through the documented QA hooks; never
inject input. Native Wayland shortcut limitations still apply.

The `autostart: clipdeck.desktop` declaration lets snapd launch the application
through its wrapper from the generated login entry. Settings shortcut commands
also use `/snap/bin/clipdeck`, preserving the profile across refreshes.
Host commands used by hooks, sounds, and system settings may be unavailable
under confinement. The `home` plug does
not grant arbitrary filesystem access. No extra filesystem exceptions are added.

No upload or release command is part of local packaging. Publishing requires
separate explicit authorization.

For Store screenshot capture, `CLIPDECK_DUMP_WINDOW_HEIGHT=700` with
`CLIPDECK_DUMP_UI` opens a taller capture window. The override is ignored
without frame dumping and accepts integer heights from 372 through 2160.
Use an isolated synthetic profile with watching and autostart disabled.
`assets/screenshots/clipdeck-history-store.png` is an unchanged 1240×700
release-frame capture, satisfying both the Store's dimension and aspect-ratio
requirements. The normal overlay size remains 1240×372.

## Local validation

The 2.1.1 candidates were built on native Ubuntu 24.04 amd64 and ARM64 runners
in [the package workflow](https://github.com/mhosseinab/clipdeck/actions/runs/43814561270).
Both the Deb and Snap builds passed all 244 contract checks and 142 workspace
tests (one desktop-keyring test ignored on headless runners). Downloaded
packages passed checksum, version, desktop metadata and architecture checks;
all 21 ELF files in each Snap matched its architecture. The autostart fix waits
briefly for the desktop keyring before opening encrypted history. Its amd64
candidate survived an actual logout/login test; physical ARM64 desktop testing
remains unverified.

The 2.1.0 amd64 candidate was built with Snapcraft 9.2.1 in LXD and installed
locally with strict confinement. Synthetic import/export, data preservation
across local reinstalls, CLI IPC, and Settings startup passed on X11 and native
Wayland. Frame dumps were pixel-probed for Settings header text. The running
executable inode matched the installed binary, and both final startup logs
were empty. Watching, hooks, sync, sounds and autostart were disabled for QA.
Workspace tests passed (140 passed, one desktop-keyring test ignored), and the
contract oracle passed all 244 checks.

The desktop-keyring test also passed when explicitly invoked. In the installed
stable candidate, a disposable profile passed confined Secret Service roundtrip,
SQLCipher conversion, external-image encryption, missing-key fail-closed behavior,
decryption and exact data restoration. The disposable key was removed after
conversion back to plaintext. Autostart-file generation passed in a disposable
HOME and the generated desktop entry passed `desktop-file-validate`; an actual
logout/login cycle was not automated. Clipboard capture/copy and shortcut
activation passed the user's final human smoke test before stable promotion.
Snapcraft reports unused-library warnings for libraries
which may be loaded dynamically; these remain bundled for desktop support.

The Settings label already uses `CARGO_PKG_VERSION`, as does `--version`.
An older `/usr/bin/clipdeck` installation reports its own version, independently
of the Snap installed at `/snap/clipdeck/current/bin/clipdeck`.

The ARM64 2.1.0 package was built on a native Launchpad ARM64 builder with
Rust 1.89.0. All 244 contract checks and 140 workspace tests passed there
(the desktop-keyring test remains ignored on the headless builder). Package
inspection verified all 21 ELF files are AArch64, plus stable/strict metadata,
the launcher, icon, license and keyboard/locale data. The Store accepted it as
revision 3 and released it to stable; amd64 remains on revision 2. ARM64 desktop
interaction has not been verified on physical ARM hardware.
