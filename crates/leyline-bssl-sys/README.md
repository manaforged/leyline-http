# leyline-bssl-sys

Raw FFI bindings to the BoringSSL build that
[`leyline-http`](https://crates.io/crates/leyline-http) uses. The crate ships
prebuilt static libraries and pregenerated bindings for four targets, so a
build needs no CMake, bindgen, Perl, or Go:

- `aarch64-apple-darwin`
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-pc-windows-msvc`

`patches/SERIES` pins the BoringSSL revision and lists the carried patches.
The patches add the TLS extensions that the Firefox profiles need
(`record_size_limit`, `delegated_credentials`) and a configurable extension
order.
[PROVENANCE.md](https://github.com/manaforged/leyline-http/blob/main/crates/leyline-bssl-sys/PROVENANCE.md)
records the source revision, the patches, and a checksum for each library.

Depend on `leyline-http` instead of this crate. Its API is outside the
`leyline-http` semver promise.

## License

`MIT AND Apache-2.0 AND BSD-3-Clause`. See
[LICENSE-BORINGSSL](https://github.com/manaforged/leyline-http/blob/main/crates/leyline-bssl-sys/LICENSE-BORINGSSL)
and the repository
[NOTICE](https://github.com/manaforged/leyline-http/blob/main/NOTICE).
