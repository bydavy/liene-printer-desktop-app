# Desktop UI control CLI

The macOS/Linux/Windows desktop app has an optional, authenticated loopback
control channel. It drives the same actions as the UI, which makes repetitive
collage/editor checks and screenshot capture deterministic without mouse
automation.

## Start the app with control enabled

```sh
cargo build -p liene-gui -p liene-cli
LIENE_CONTROL_FILE=/tmp/open-liene-control.json \
  ./target/debug/open-desktop --control
```

The app writes a discovery file containing an ephemeral loopback port,
a random per-process token, and its PID. On Unix the file is mode `0600`.
The server binds only to `127.0.0.1` and is off unless `--control` is passed.
`LIENE_CONTROL_FILE` is optional; without it the app uses the OS temp directory.

The `open-liene-control` binary lives next to `open-desktop`:

```sh
./target/debug/open-liene-control --file /tmp/open-liene-control.json state
```

Use `--file` (or `LIENE_CONTROL_FILE`) for both commands when you override the
discovery path.

## Commands

```sh
# Inspect and load
open-liene-control state
open-liene-control image open photo.jpg
open-liene-control collage open a.jpg b.jpg c.jpg

# Collage layout
open-liene-control collage layout h3
open-liene-control collage borders off
open-liene-control collage randomize

# One photo: zoom, pan, rotation, replace
open-liene-control collage frame set 0 --zoom 2.0 --pan-x 0.5 --pan-y -0.2 --rotation 90
open-liene-control collage frame rotate 0
open-liene-control collage frame reset 0
open-liene-control collage frame replace 0 new-photo.jpg
open-liene-control collage frame open 0
open-liene-control collage frame close

# Print review / print (printer must be connected for printing)
open-liene-control collage print-preview
open-liene-control collage print --copies 2

# Screenshot and quit
open-liene-control screenshot /tmp/collage.png
open-liene-control quit
```

`state` prints a JSON snapshot including the selected layout, borders, photo
paths, and per-photo `frames` (`zoom`, `pan_x`, `pan_y`, `rotation`). Actions
return a state snapshot too, so a test can assert the result without scraping
the UI.

## Suggested regression test

```sh
CONTROL=/tmp/open-liene-control.json
LIENE_CONTROL_FILE="$CONTROL" LIENE_NO_CONNECT=1 \
  ./target/debug/open-desktop --control &
APP_PID=$!

./target/debug/open-liene-control --file "$CONTROL" collage open a.jpg b.jpg c.jpg
./target/debug/open-liene-control --file "$CONTROL" collage layout h3
./target/debug/open-liene-control --file "$CONTROL" collage borders off
./target/debug/open-liene-control --file "$CONTROL" collage frame set 0 \
  --zoom 2 --pan-x 1 --rotation 90
./target/debug/open-liene-control --file "$CONTROL" screenshot /tmp/collage.png
./target/debug/open-liene-control --file "$CONTROL" quit
wait "$APP_PID"
```

The screenshot uses the app's offscreen framebuffer, so it does not depend on
window position or the host screen.

## Security

- Control is explicitly opt-in with `--control`.
- It binds to loopback only.
- Requests must include the token from the discovery file.
- On Unix the discovery file is readable only by the user.
- The discovery file is removed on app exit.
