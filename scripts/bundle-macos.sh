#!/usr/bin/env bash
# Build a self-contained macOS .app bundle.
#
# Host build:      ./scripts/bundle-macos.sh
# Universal build: ./scripts/bundle-macos.sh --universal
#
# VERSION=0.3.2 can be supplied by the release workflow.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

UNIVERSAL=0
if [[ "${1:-}" == "--universal" ]]; then
    UNIVERSAL=1
fi

VERSION="${VERSION:-0.3.2}"
VERSION="${VERSION#v}"

BIN_DIR="$ROOT/target/liene-bundle"
rm -rf "$BIN_DIR"
mkdir -p "$BIN_DIR"

if [[ "$UNIVERSAL" == "1" ]]; then
    command -v rustup >/dev/null || {
        echo "error: --universal requires a rustup-managed toolchain (Rust 1.95+)" >&2
        exit 1
    }
    CARGO_BIN="${CARGO:-$(rustup which cargo)}"
    # Invoke the toolchain's real compiler, not the rustup shim. The shim can
    # start a channel sync / rust-src download while cargo is building, which
    # races with the build and fails with a rustup rename error.
    export RUSTC="$(rustup which rustc)"

    installed_targets="$(rustup target list --installed)"
    for target in aarch64-apple-darwin x86_64-apple-darwin; do
        if ! grep -qx "$target" <<< "$installed_targets"; then
            rustup target add "$target"
        fi
    done

    "$CARGO_BIN" build --release --locked --target aarch64-apple-darwin -p liene-cli -p liene-gui
    "$CARGO_BIN" build --release --locked --target x86_64-apple-darwin -p liene-cli -p liene-gui

    lipo -create \
        -output "$BIN_DIR/liene-desktop-app" \
        "target/aarch64-apple-darwin/release/liene-desktop-app" \
        "target/x86_64-apple-darwin/release/liene-desktop-app"
    lipo -create \
        -output "$BIN_DIR/liene-n200" \
        "target/aarch64-apple-darwin/release/liene-n200" \
        "target/x86_64-apple-darwin/release/liene-n200"
else
    CARGO_BIN="${CARGO:-cargo}"
    "$CARGO_BIN" build --release --locked -p liene-cli -p liene-gui
    cp "target/release/liene-desktop-app" "$BIN_DIR/liene-desktop-app"
    cp "target/release/liene-n200" "$BIN_DIR/liene-n200"
fi

APP="$ROOT/dist/Liene Desktop App.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$BIN_DIR/liene-desktop-app" "$APP/Contents/MacOS/liene-desktop-app"
# The GUI spawns this helper so IOBluetooth lives in a process whose main run
# loop services its callbacks.
cp "$BIN_DIR/liene-n200" "$APP/Contents/MacOS/liene-n200"

if [[ -f "$ROOT/assets/AppIcon.icns" ]]; then
    cp "$ROOT/assets/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"
fi

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleExecutable</key>
    <string>liene-desktop-app</string>
    <key>CFBundleIdentifier</key>
    <string>io.github.bydavy.liene-printer-desktop-app</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>Liene Desktop App</string>
    <key>CFBundleDisplayName</key>
    <string>Liene Desktop App</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.photography</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
    <key>NSBluetoothAlwaysUsageDescription</key>
    <string>Liene Desktop App needs Bluetooth access to discover and print to your photo printer.</string>
    <key>NSHumanReadableCopyright</key>
    <string>MIT-licensed unofficial client; not affiliated with Liene or Hannto.</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
</dict>
</plist>
PLIST

# Ad-hoc signature: enough for local use. Distribution to other Macs needs a
# Developer ID certificate and notarization.
codesign --force --sign - "$APP/Contents/MacOS/liene-n200" >/dev/null
codesign --force --deep --sign - "$APP" >/dev/null

echo "Built: $APP (version $VERSION)"
