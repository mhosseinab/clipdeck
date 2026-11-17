# Development

Build, test, and package ClipDeck from source. Run commands from the repository
root unless noted otherwise. For installation and everyday use, see the
[README](../README.md).

## Build and run

Requires Rust 1.89 or newer (use current stable), a C build toolchain, `make`,
Perl, and `pkg-config` / `libdbus-1-dev` on Ubuntu. SQLCipher and OpenSSL are
built through the Rust dependency graph. A graphical Linux session
is required to run the app. GNOME users need a tray extension for tray visibility;
the CLI works without a tray.

```bash
bash scripts/check.sh linux
./apps/linux/target/release/clipdeck run
```

## Package a Debian installer and tarball

Requires `cargo-deb` 3.8.0, `dpkg-deb`, `desktop-file-validate`, `appstreamcli`, `tar` and
`sha256sum` in addition to the build tools. On Ubuntu, the desktop validator is
provided by `desktop-file-utils`, and `appstreamcli` by `appstream`.
Install tooling separately if missing.

```bash
bash scripts/package.sh
# -> dist/candidate/*.deb, *.tar.gz, SHA256SUMS
```

This command runs checks, builds the binary, validates both package payloads
and writes checksums. It does not install, tag, upload or publish anything.
A package built on a newer distro may not run on older Ubuntu versions.
Build on the oldest supported distro when preparing portable packages.
The tarball contains `usr/bin/clipdeck`, desktop/icon resources and licenses;
it is an extracted prefix layout, not an AppImage or self-contained runtime.

The manual **Package candidates** GitHub Actions workflow builds Deb, tarball
and Snap artifacts on native Ubuntu 24.04 amd64 and ARM64 runners. It runs the
same package checks and retains artifacts for 14 days. It does not publish
packages or create releases.

## Package a Snap

See the [Snap packaging guide](../snap/README.md) for the Snapcraft/LXD build,
local installation, confinement checks, and screenshot capture. Building a
package does not publish it.

## Install or publish the Rust crates

The registry packages are `clipdeck-core` (library) and `clipdeck-app` (Linux
application). The application installs the `clipdeck` executable:

```bash
cargo install clipdeck-app --locked
```

Use the build prerequisites above. Cargo installs the executable without the
desktop entries provided by the Snap and Debian packages.

For an authorized crates.io publication, authenticate with `cargo login`, then
verify and publish both packages from a clean checkout using a current Cargo:

```bash
cargo publish --manifest-path apps/linux/Cargo.toml --dry-run -p clipdeck-core -p clipdeck-app
cargo publish --manifest-path apps/linux/Cargo.toml -p clipdeck-core -p clipdeck-app
```

Cargo stages and verifies both archives, then publishes the core dependency
before the application. Keep their versions and the application's exact core
dependency requirement aligned. Repository integration tests remain in the full
checkout; the core registry archive excludes tests that require external fixtures
or application assets. The independent specification crate is not published.

## Checks and profiles

```bash
bash scripts/check.sh spec        # fixture oracle
bash scripts/check.sh linux-core  # oracle + Rust core fixtures
bash scripts/check.sh linux       # oracle + workspace tests + release build
```

Stop the daemon before using CLI `import` or `export`; both require exclusive
profile ownership. Back up the full profile before installing a candidate.
CLI `export` contains records, not image-file bytes or settings.

The default profile is `$XDG_DATA_HOME/clipdeck`, falling back to
`~/.local/share/clipdeck` when `XDG_DATA_HOME` is unset or not absolute.
Set `CLIPDECK_USER_DATA` to
use a separate profile, and pass the same value to its daemon and CLI commands.
Each profile has a private `.ipc/control.sock`; multiple profiles have independent
control endpoints. Profiles and control directories use owner-only permissions.
Keep profile paths short enough for Linux's Unix-domain socket path limit.

These defaults and overrides apply to native builds. The Snap launcher sets
`CLIPDECK_USER_DATA` to `$SNAP_USER_COMMON/profile` for every command; an
environment override on `snap run clipdeck` does not select another profile.

ClipDeck stores history in `clipdeck.sqlite3`. The overlay loads small pages of card
summaries, fetching full clip contents when needed. Database and thumbnail
caches have separate budgets. Legacy `clipdeck-store.json` history is discarded
instead of migrated. Existing SQLite history and images are preserved when
adopting a profile layout or changing encryption mode. For an existing storage
directory, a missing policy or active database is reported as an error instead
of silently starting fresh.

On first launch, choose encrypted history or continue without encryption before
clipboard watching starts. Encryption uses SQLCipher with a key stored in the
OS keyring. Losing that key makes encrypted history unreadable. Settings can
enable or disable encryption after a restart while preserving the current
database and images. A locked or unavailable keyring never causes plaintext
fallback. Images stay in external files, encrypted when encrypted history is
enabled.
Encrypted history requires a Secret Service keyring and `secret-tool`
(`libsecret-tools` on Ubuntu). Plaintext history works without them.

For AI-assisted development, see [the agent setup guide](agentic-development.md)
for project rules, reusable skills, optional agents, and scoped check commands.

## Repository map

| Path | Purpose |
| --- | --- |
| `apps/linux/crates/app/` | Native UI, clipboard worker, tray and desktop integration |
| `apps/linux/crates/core/` | Store, classification, settings, sync and fixture tests |
| `spec/` | Behavior contract, fixtures, schemas and reference oracle |
| `scripts/` | Verification and local candidate packaging |
| `docs/` | Build, contributor and agent workflow guides |
| `.agents/`, `.codex/`, `AGENTS.md` | Repository-scoped agent skills and rules |

## Storage and format compatibility

Native persistence uses SQLite. JSON export/import and JSONL folder sync remain
available; their formats are described in `spec/schema/`. Quit ClipDeck before
copying or restoring a profile, and include `clipdeck-security.json` and its
active storage directory with the database, any SQLite sidecars and images.
Encrypted profiles also require their OS
keyring key associated with the canonical profile path. Moving a profile does
not automatically move that keyring association. JSON exports and the system
clipboard are plaintext. JSONL folder
sync is disabled for encrypted profiles; encryption does not protect old
exports or files. CLI `export` contains full clip records but does not include
image-file bytes, settings or encryption keys.
