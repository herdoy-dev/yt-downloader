#!/bin/bash
# Builds Vidlasso.app and packs it into a compressed Vidlasso.dmg.
set -euo pipefail
cd "$(dirname "$0")"

BIN=src-tauri/binaries/yt-dlp-aarch64-apple-darwin
if [ ! -x "$BIN" ]; then
  echo "Downloading yt-dlp…"
  mkdir -p src-tauri/binaries
  curl -fL -o "$BIN" https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos
  chmod +x "$BIN"
fi

npx --yes @tauri-apps/cli@latest build --bundles app

APP=src-tauri/target/release/bundle/macos/Vidlasso.app
STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
rm -f Vidlasso.dmg
hdiutil create -volname Vidlasso -srcfolder "$STAGE" -ov -format ULMO Vidlasso.dmg
rm -rf "$STAGE"
ls -lh Vidlasso.dmg
