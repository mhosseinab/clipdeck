#!/usr/bin/env bash
# Verify without launching the clipboard daemon or modifying user settings.
set -euo pipefail
usage() { echo 'Usage: bash scripts/check.sh {spec|linux-core|linux}'; }
[[ $# -eq 1 ]] || { usage >&2; exit 2; }
case "$1" in
  -h|--help) usage; exit 0 ;;
  spec|linux-core|linux) ;;
  *) usage >&2; exit 2 ;;
esac
cd "$(dirname "${BASH_SOURCE[0]}")/.."
cargo run --locked --manifest-path apps/linux/Cargo.toml -p clipdeck-spec --
case "$1" in
  linux-core) cargo test --locked --manifest-path apps/linux/Cargo.toml -p clipdeck-core ;;
  linux)
    cargo test --locked --manifest-path apps/linux/Cargo.toml --workspace
    cargo build --locked --release --manifest-path apps/linux/Cargo.toml --package clipdeck-app
    ;;
esac
