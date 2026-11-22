# Changelog

## [Unreleased]

### Added

- Collage mode for 2–4 photos with vertical, horizontal, focused, and grid
  layouts, a live preview, and a final print-preview window with copies.

## [0.3.2] - 2026-11-22

### Fixed

- Battery status is shown as the printer's actual 0–4 level (`Battery full`,
  `75%`, etc.) instead of misreading level `4` as 4%.

### Changed

- Print resolution is fixed at 1200×1800 in the GUI and CLI. The 600×900
  selector and the `--width`/`--height` overrides were removed.
- The photo editor and print preview now have square corners so the framing
  matches the printed paper.
- The two rotate buttons are replaced by a single **Rotate 90°** button.
- The redundant **2:3 crop** button is removed; **Lock 2:3** performs the
  same snap.

## [0.3.1] - 2026-11-21

### Added

- Light and dark themes that follow the system appearance, with a dedicated
  palette for each (including theme-aware cards, status colors, buttons, and
  error banners).

### Fixed

- The app no longer falls back to unstyled white panels in Light Mode; both
  themes follow the OS appearance and the custom design.
- Long connection errors now wrap within the error banner instead of
  squeezing and truncating the **Resume printer** and **Dismiss** buttons.

### Changed

- The macOS disk image is now named `Liene-Desktop-App-<version>-macos.dmg`,
  matching the platform suffix used by the Linux and Windows packages.

## [0.3.0] - 2026-11-21

### Changed

- Renamed the desktop app to **Liene Desktop App** (binary
  `liene-desktop-app`; release assets now `Liene-Desktop-App-*`).
- Default print quality is now 1200×1800 (2× the native 300 dpi output);
  600×900 remains selectable. CLI `print` and `prepare` also default to
  1200×1800.
- Default framing is now **Fit** (contain, white borders) instead of Fill
  (cover). Fill remains selectable in the app and via `--fit cover`.
- The macOS bundle, Linux archive, Windows zip, and DMG names follow the new
  app name.

## [0.2.0] - 2026-11-21

### Added

- Experimental Linux (BlueZ RFCOMM) and Windows (WinRT RFCOMM) transports.
  These are compile-tested and packaged in CI but have not been hardware-tested
  with an N200 Pro yet. Testers wanted.
- Linux `.tar.gz` and Windows `.zip` packaging scripts; CI artifacts and
  release assets (clearly marked untested).
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
