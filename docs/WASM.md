# Why there is no WASM build

The UI, protocol, and edit model are portable, but a browser **cannot print to
this printer**, so there is no WASM release:

- The N200 Pro carries print data over **Bluetooth Classic SPP (RFCOMM)**.
  No browser API exposes Bluetooth Classic SPP. Web Bluetooth only supports
  BLE GATT; WebUSB and WebSerial do not expose RFCOMM.
- The printer's USB-C port is charging-only, so there is no USB data path to
  use either.
- The vendor Android app uses BLE only to trigger a reverse SPP connection
  (`A_OUT_<phone-mac>` on `fe95/0013`); that is not a print-data service. A
  pure browser build would need a **BLE GATT print service** on the printer.
  That has not been found or verified on the N200 Pro, and finding one would be
  a separate reverse-engineering and hardware-validation project.
- Even if a GATT data path existed, Web Bluetooth is Chromium-only (no Safari
  or iOS), requires HTTPS and a user gesture, hides MAC addresses, and cannot
  auto-connect. Chunking/ACK behavior would also need to be revalidated for
  BLE (the SPP path uses 892-byte chunks).

Publishing a WASM build today would produce a browser editor that cannot send
a print. The realistic options are:

1. **WASM UI + a local native bridge** — keep the existing Rust SPP transport
   in a small localhost helper/WebSocket server and use the WASM app as its
   frontend. This is a web UI, not a standalone browser app.
2. **BLE GATT transport** — only if the printer exposes a usable print-data
   GATT service; unverified, and still limited to Chromium browsers.
3. **Remote native host** — run the native app near the printer and control it
   over the network.

To port the current code to WASM we would also need to replace the native
transport and pairing, `std::process` helper architecture, blocking threads,
Tokio timers, filesystem image loading, and native file dialogs with Web
Bluetooth/local-bridge, browser File APIs, browser timers, and a WASM-friendly
event loop. The protocol and editing/preview crates are already reusable.

If you know of a BLE GATT data path on any Liene/Hannto model, please open a
[Printer support report](https://github.com/bydavy/liene-printer-desktop-app/issues/new?template=printer-support.md);
that is the only route to a pure browser app.
