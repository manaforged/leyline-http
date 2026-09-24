# leyline-bssl-sys

Raw FFI bindings to the BoringSSL build that
[`leyline-http`](https://crates.io/crates/leyline-http) uses. The crate is a
trimmed fork of Cloudflare's
[`boring-sys`](https://github.com/cloudflare/boring). It ships the BoringSSL
source, applies the patches in `patches/`, builds it with CMake, and generates
the bindings with `bindgen`.

Supported targets:

- `aarch64-apple-darwin`
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-pc-windows-msvc`

The build needs CMake 3.22 or later, a C and C++ compiler, and libclang. On
Windows it also needs the MSVC build tools and NASM. Every export carries the
`LEYLINE` symbol prefix, so the library can share a binary with `openssl-sys`
or another BoringSSL.

[PROVENANCE.md](https://github.com/manaforged/leyline-http/blob/main/crates/leyline-bssl-sys/PROVENANCE.md)
records the upstream base, the BoringSSL revision, and the patches.

Depend on `leyline-http` instead of this crate. Its API is outside the
`leyline-http` semver promise.

## License

`MIT AND Apache-2.0 AND BSD-3-Clause`. See
[LICENSE-BORINGSSL](https://github.com/manaforged/leyline-http/blob/main/crates/leyline-bssl-sys/LICENSE-BORINGSSL)
and the repository
[NOTICE](https://github.com/manaforged/leyline-http/blob/main/NOTICE).
