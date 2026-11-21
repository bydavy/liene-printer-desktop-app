# How it works

The printer speaks a framed binary protocol over **Bluetooth Classic SPP**
(RFCOMM channel 1), not BLE. JSON control messages and image chunks share the
same framing:

```text
0x7E | version | reserve | connectType | interactive | encoding
     | terminalId u32 | msgSn u32 | pkgTotal u16 | pkgNum u16
     | attributes u16 | body | checksum u8 | 0x7E
```

A print job is `print-job` JSON → 892-byte file chunks (each prefixed with the
4-byte job id) → automatic start. Status uses `get-prop`; job tracking uses
`get-job-info`. The exact fields, constants, and observed responses are in
[PROTOCOL.md](PROTOCOL.md).

## Architecture

```text
crates/liene-protocol/      portable frames, JSON-RPC, chunking (no platform code)
crates/liene-client/        SPP transport, pairing, editing, image pipeline, daemon
crates/liene-cli/           command-line driver + hidden JSON-line `serve` mode
crates/liene-gui/           egui desktop app
vendor/waterkit-bluetooth/  patched transport (direct RFCOMM channel 1)
docs/PROTOCOL.md            recovered protocol reference
docs/WASM.md                why a browser/WASM build cannot print
```

The GUI spawns the CLI in `serve` mode and talks to it over JSON lines. On
macOS this keeps IOBluetooth callbacks on a process whose main run loop
services them; winit/eframe's macOS loop does not reliably do so. The same
helper boundary is used on Linux and Windows, and both binaries are shipped
together in the macOS `.app` and the Linux/Windows packages.
