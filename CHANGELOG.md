# Changelog

## [0.4.0] - 2026-11-22

### Added

- `egui_kittest` screenshot and interaction tests for the photo editor,
  collage panel, frame editor, and print preview, plus headless tests for the
  UI control channel (token rejection, request forwarding, and discovery-file
  permissions).

### Changed

- **Breaking:** the desktop CLI is renamed from `liene-n200` to `liene-cli`,
  matching the crate and the multi-model support. The GUI launches
  `liene-cli serve` as its Bluetooth helper. There is no `liene-n200` alias.

### Fixed

- Printer status dots are painted instead of using `●`/`○` text glyphs, which
  are missing from egui's bundled fonts on some systems.
- Copies fields now expose accessible labels.

## [0.3.3] - 2026-11-22

### Added

- Collage mode for 2–4 photos with vertical, horizontal, focused, and grid
  layouts, a live preview, and a final print-preview window with copies.
- Collage layout options use white/grey diagram icons instead of text labels.
- Per-photo frame editor with zoom, pan, 90° rotation, replacement, and reset.
- A Borders checkbox (on by default) and an explicit Randomize button.
- Opt-in authenticated loopback UI control channel (`--control`) and
  `liene-control` CLI for deterministic collage/editor tests and screenshots.

### Fixed

- The collage preview now selects the same area of each photo as the frame
  editor and the print. Horizontal panning also works at zoom 1, where the
  preview previously ignored it.

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
