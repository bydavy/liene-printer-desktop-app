#!/usr/bin/env bash
# Build an UNTESTED portable Linux x86_64 bundle.
#
# The Linux transport compiles and is CI-built, but has not been hardware
# tested with an N200 Pro. Pair the printer with BlueZ first:
#   bluetoothctl --timeout 30 pair <MAC> trust <MAC> connect <MAC>
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="${VERSION:-0.1.0}"
VERSION="${VERSION#v}"

cargo build --release --locked -p liene-cli -p liene-gui

NAME="Liene-N200-Pro-${VERSION}-linux-x86_64"
STAGE="$ROOT/dist/$NAME"
rm -rf "$STAGE"
mkdir -p "$STAGE"

cp target/release/liene-n200 target/release/liene-n200-app "$STAGE/"
cp README.md LICENSE "$STAGE/"
cp assets/AppIcon.png "$STAGE/liene-n200-pro.png"

cat > "$STAGE/run.sh" <<'RUN'
#!/usr/bin/env bash
# Keep the helper next to the GUI binary.
cd "$(dirname "$0")"
exec ./liene-n200-app "$@"
RUN
chmod +x "$STAGE/run.sh"

cat > "$STAGE/README-UNTESTED.txt" <<'TXT'
UNTESTED LINUX BUILD

This Linux build is provided for testing. It is compile-checked in CI but has
not been verified on real N200 Pro hardware.

1. Install BlueZ and pair the printer:
     bluetoothctl --timeout 30 pair <MAC>
     bluetoothctl trust <MAC>
     bluetoothctl connect <MAC>
2. Keep liene-n200 and liene-n200-app in the same directory.
3. Run ./run.sh (or ./liene-n200-app).

If it works, please open an issue with your distro, BlueZ version, printer
model/firmware, and a `liene-n200 --debug status` log. If it fails, that log is
equally useful. Help wanted: https://github.com/bydavy/liene-printer-desktop-app/issues
TXT

tar -C "$ROOT/dist" -czf "$ROOT/dist/$NAME.tar.gz" "$NAME"
(
    cd "$ROOT/dist"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$NAME.tar.gz" > "$NAME.tar.gz.sha256"
    else
        shasum -a 256 "$NAME.tar.gz" > "$NAME.tar.gz.sha256"
    fi
)
echo "Built: $ROOT/dist/$NAME.tar.gz (untested)"
