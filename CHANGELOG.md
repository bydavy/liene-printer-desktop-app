# Changelog

## [0.1.0] - 2026-11-21

Initial public release.

- Native macOS app (Rust/egui) and CLI for the Liene Pearl N200 Pro (DMP130).
- Bluetooth Classic SPP transport, pairing, status/info queries, job tracking,
  resume/cancel, and raw protocol probes.
- Printing over the Hannto frame protocol with 892-byte chunk upload and
  paced flow control.
- EXIF orientation, rotate, crop, 2:3 aspect lock, zoom/framing, Fill/Fit, and
  a live print preview with white borders.
- Universal macOS app bundle and DMG packaging plus GitHub Actions CI/release
  workflows.
- Protocol reference in `docs/PROTOCOL.md`.
