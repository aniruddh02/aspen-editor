#!/usr/bin/env bash
# Download the standalone Lightroom MCP binary so Aspen Image Editing
# does not require Homebrew or a system Node.js install.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT_DIR="$ROOT/src-tauri/resources"
OUT="$OUT_DIR/lightroom-mcp"
VERSION="${LIGHTROOM_MCP_VERSION:-v0.9.0}"
ARCH="$(uname -m)"

case "$ARCH" in
  arm64|aarch64) ASSET="lightroom-mcp-darwin-arm64" ;;
  x86_64) ASSET="lightroom-mcp-darwin-x64" ;;
  *)
    echo "Unsupported macOS arch: $ARCH" >&2
    exit 1
    ;;
esac

URL="https://github.com/Automaat/lightroom-mcp/releases/download/${VERSION}/${ASSET}"
mkdir -p "$OUT_DIR"

if [[ -x "$OUT" ]]; then
  echo "Lightroom MCP binary already present: $OUT"
  exit 0
fi

echo "Downloading $URL"
curl -fL --retry 3 -o "$OUT.tmp" "$URL"
chmod +x "$OUT.tmp"
# Clear quarantine so Gatekeeper doesn't flag the helper binary.
xattr -cr "$OUT.tmp" 2>/dev/null || true
mv "$OUT.tmp" "$OUT"
echo "Installed $OUT"
