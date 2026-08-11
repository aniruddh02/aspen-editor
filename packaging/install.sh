#!/usr/bin/env bash
# Install Aspen.app into /Applications (no Finder Services in v1).
set -euo pipefail

APP_SRC="${1:-}"
if [[ -z "$APP_SRC" ]]; then
  # Try common build output
  if [[ -d "src-tauri/target/release/bundle/macos/Aspen.app" ]]; then
    APP_SRC="src-tauri/target/release/bundle/macos/Aspen.app"
  else
    echo "Usage: $0 /path/to/Aspen.app"
    exit 1
  fi
fi

if [[ ! -d "$APP_SRC" ]]; then
  echo "Not found: $APP_SRC"
  exit 1
fi

DEST="/Applications/Aspen.app"
echo "Installing $APP_SRC → $DEST"
rm -rf "$DEST"
cp -R "$APP_SRC" "$DEST"

echo "Clearing quarantine attributes (Gatekeeper)…"
xattr -cr "$DEST" || true

echo ""
echo "Installed. Open Aspen from Applications / Spotlight."
echo "First launch: System Settings → Privacy & Security → Open Anyway,"
echo "or Right-click Aspen → Open."
echo "If macOS still says the app is damaged (older download):"
echo "  xattr -cr \"$DEST\""
echo "Docs: https://github.com/aniruddh02/aspen-editor#readme"
