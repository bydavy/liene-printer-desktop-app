# Changelog

## [Unreleased]

### Added

- Experimental Linux (BlueZ RFCOMM) and Windows (WinRT RFCOMM) transports.
  These are compile-tested and packaged in CI but have not been hardware-tested
  with an N200 Pro yet. Testers wanted.
- Linux `.tar.gz` and Windows `.zip` packaging scripts and CI artifacts.
- CI matrix for macOS, Linux, and Windows, plus a macOS universal-bundle build.
- Printer-support issue template and a "help wanted" section listing related
  Liene/Hannto models that need testers.
- Linux fallback to RFCOMM channel 1 when SDP/`sdptool` is unavailable.
- Windows GUI subsystem and no-console helper launch.

### Fixed

- Avoid a rustup shim race during universal builds by invoking the toolchain's
  real `rustc` and skipping target installation when targets are present.
- Disabled release LTO to keep macOS CI builds within runner memory.

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
