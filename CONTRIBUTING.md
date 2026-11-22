# Contributing

Thanks for helping improve the Liene Desktop App client.

## Good first contributions

- Hardware reports on the N200 Pro and related Hannto printers (model, PID,
  firmware, macOS version, what worked/failed).
- Protocol captures and documentation improvements.
- Linux/Windows Bluetooth transports.
- UI/UX polish, accessibility, and documentation.

## Development setup

Requirements: macOS 13+, Xcode Command Line Tools, Rust 1.95+.

```sh
cargo build --release
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

The app builds with `cargo run -p liene-gui`. A double-clickable bundle and DMG
are produced by:

```sh
./scripts/bundle-macos.sh
VERSION=0.3.2 ./scripts/make-dmg.sh
```

Hardware testing requires the printer; unit tests must pass without it.

## Pull requests

- Keep changes focused and explain the hardware/software context.
- Run format, clippy, and tests.
- Update `docs/PROTOCOL.md` for protocol changes and the README for user-facing
  changes.
- Do not commit files under `reverse/` or `tools/`. They contain downloaded
  vendor binaries, decompiled output, and reverse-engineering toolchains that
  must not be redistributed.
- Do not add vendor app code, firmware, icons, or other proprietary assets.

## Reporting issues

Include:

- Printer model and firmware version (`liene-n200 info`).
- macOS version and app version/commit.
- Steps to reproduce.
- Relevant log output (`--debug` or the app's Log panel).
- A photo-paper/ribbon state if the problem is print-related.

## Security and privacy

Remove MAC addresses, serial numbers, account identifiers, and photo paths from
logs or screenshots before posting them publicly. This project talks only to
the local Bluetooth printer and does not send data over the network.
