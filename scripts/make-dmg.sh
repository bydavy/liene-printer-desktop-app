#!/usr/bin/env bash
# Package dist/Liene Desktop App.app into a compressed DMG with an
# /Applications shortcut for drag-and-drop installation.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="${VERSION:-0.4.0}"
VERSION="${VERSION#v}"

APP="$ROOT/dist/Liene Desktop App.app"
if [[ ! -d "$APP" ]]; then
    echo "error: $APP not found; run scripts/bundle-macos.sh first" >&2
    exit 1
fi

STAGE="$ROOT/build/dmg"
rm -rf "$STAGE"
mkdir -p "$STAGE"
ditto "$APP" "$STAGE/Liene Desktop App.app"
ln -s /Applications "$STAGE/Applications"

DMG="$ROOT/dist/Liene-Desktop-App-${VERSION}-macos.dmg"
rm -f "$DMG"
hdiutil create \
    -volname "Liene Desktop App" \
    -srcfolder "$STAGE" \
    -ov \
    -format UDZO \
    "$DMG" >/dev/null

# Sign the disk image itself ad-hoc (not a substitute for notarization).
codesign --force --sign - "$DMG" >/dev/null 2>&1 || true

(
    cd "$ROOT/dist"
    shasum -a 256 "$(basename "$DMG")" > "$(basename "$DMG").sha256"
)

echo "Built: $DMG"
cat "$DMG.sha256"
