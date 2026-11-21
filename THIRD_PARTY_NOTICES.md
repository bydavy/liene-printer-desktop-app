# Third-party notices

This project is MIT licensed (see `LICENSE`). It depends on third-party Rust
crates whose licenses are recorded in `Cargo.lock` and in each crate's
distribution.

## Vendored `waterkit-bluetooth`

`vendor/waterkit-bluetooth` is a copy of
[`waterkit-bluetooth` 0.3.0](https://crates.io/crates/waterkit-bluetooth)
(MIT OR Apache-2.0), included because this printer needs a small change:

- It opens the Liene SPP service on RFCOMM channel 1 directly. A
  UUID-filtered `performSDPQuery` on this firmware returns success but never
  invokes its completion callback.
- It removes a `rfcommChannelWriteComplete:refcon:` delegate override that the
  current `objc2-io-bluetooth` protocol does not declare (which trips objc2's
  debug assertion).

Upstream license files are included in that directory
(`LICENSE-MIT`, `LICENSE-APACHE`).

## Protocol acknowledgements

The protocol findings were informed by prior community work on the Liene
PixCut S1:

- <https://github.com/sincethestudy/liene-pixcut-s1-api>
- <https://github.com/eastbaymakersclub/pixcut-s1>

No source code from those repositories is included here.

## Trademarks

Liene, Pearl, Hannto, and any related marks are the property of their
respective owners. This project is unofficial and is not affiliated with,
endorsed by, or sponsored by Liene or Hannto.
