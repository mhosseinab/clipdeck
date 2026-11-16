#!/usr/bin/env bash
# Build local candidates only. Never installs, tags, uploads or publishes.
set -euo pipefail
[[ $# -eq 0 ]] || { echo 'Usage: bash scripts/package.sh' >&2; exit 2; }
cd "$(dirname "${BASH_SOURCE[0]}")/.."
for tool in cargo dpkg-deb desktop-file-validate appstreamcli sha256sum tar; do
  command -v "$tool" >/dev/null || { echo "Missing packaging tool: $tool" >&2; exit 1; }
done
cargo deb --version >/dev/null
bash scripts/check.sh linux
target_dir="$(cargo run --quiet --locked --manifest-path apps/linux/Cargo.toml -p clipdeck-spec -- --print-target-dir)"
binary="$target_dir/release/clipdeck"
output="$PWD/dist/candidate"
app_id="io.github.mhosseinab.clipdeck"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
mkdir -p "$output"
# cargo-deb resolves target/release relative to the workspace's Cargo target dir.
cargo deb --locked --no-build --manifest-path apps/linux/crates/app/Cargo.toml --output "$stage/clipdeck.deb"
dpkg-deb --extract "$stage/clipdeck.deb" "$stage/payload"
cmp "$binary" "$stage/payload/usr/bin/clipdeck"
cmp apps/linux/crates/app/assets/icon.png "$stage/payload/usr/share/icons/hicolor/256x256/apps/$app_id.png"
desktop-file-validate "$stage/payload/usr/share/applications/$app_id.desktop"
appstreamcli validate --no-net --strict "$stage/payload/usr/share/metainfo/$app_id.metainfo.xml"
appstreamcli validate-tree --no-net --strict "$stage/payload"
[[ -f "$stage/payload/usr/share/doc/clipdeck/copyright" ]]
[[ -f "$stage/payload/usr/share/doc/clipdeck/OFL-NotoSansArabic.txt" ]]
[[ -f "$stage/payload/usr/share/doc/clipdeck/THIRD-PARTY-TEXT-LICENSES.txt" ]]
version="$(dpkg-deb --field "$stage/clipdeck.deb" Version)"
arch="$(dpkg-deb --field "$stage/clipdeck.deb" Architecture)"
[[ "$("$binary" --version)" == "ClipDeck ${version%-*}" ]]
grep -F "<release version=\"${version%-*}\"" "$stage/payload/usr/share/metainfo/$app_id.metainfo.xml" >/dev/null
name="clipdeck_${version}_${arch}"
mkdir "$stage/$name"
cp -a "$stage/payload/usr" "$stage/$name/"
cp README.md LICENSE "$stage/$name/"
tar -czf "$stage/$name.tar.gz" -C "$stage" "$name"
# Extract and compare: a manifest listing alone cannot prove a usable payload.
mkdir "$stage/unpacked"
tar -xzf "$stage/$name.tar.gz" -C "$stage/unpacked"
cmp "$binary" "$stage/unpacked/$name/usr/bin/clipdeck"
mv "$stage/clipdeck.deb" "$output/$name.deb"
mv "$stage/$name.tar.gz" "$output/"
(cd "$output" && sha256sum "$name.deb" "$name.tar.gz" > SHA256SUMS && sha256sum --check SHA256SUMS)
dpkg-deb --field "$output/$name.deb" Package Version Architecture Depends
printf 'Local candidates: %s\nNothing was installed or published.\n' "$output"
