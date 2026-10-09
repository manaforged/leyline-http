# leyline-bssl-sys

Raw FFI bindings to the BoringSSL build that
[`leyline-http`](https://crates.io/crates/leyline-http) uses. The crate is a
trimmed fork of Cloudflare's
[`boring-sys`](https://github.com/cloudflare/boring). It ships the BoringSSL
source, applies the patches in `patches/`, and compiles it with the `cc`
crate from BoringSSL's own source lists, so no CMake, Go, or Perl is needed. The
bindings for each supported target are pre-generated in `bindings/`. Enable the
`bindgen` feature to generate them at build time; that path needs libclang.

Supported targets:

- `aarch64-apple-darwin`
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-unknown-linux-musl`
- `aarch64-unknown-linux-musl`
- `x86_64-pc-windows-msvc`

The build needs Git and a C and C++ compiler. On Windows that is the MSVC
build tools; NASM is optional, because the crate ships the assembled objects
in `prebuilt/` and uses NASM only when it is on `PATH`. On musl it needs a
musl C and C++ toolchain, such as `x86_64-linux-musl-g++` from musl.cc. Every export carries the
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
