# Features and targets

## Cargo features

The package is `leyline-http` and the library is `leyline`. These features are
on by default: `charset`, `compression-gzip`, `compression-brotli`,
`compression-deflate`, `compression-zstd`, `multipart`, `stream`, `websocket`,
and `http3`. The cookie jar and system trust-store loading always compile.

| Feature | Default | What it changes |
| --- | --- | --- |
| `charset` | yes | Adds `encoding_rs`. Without it, `Response::text` falls back to lossy UTF-8 instead of honoring the `Content-Type` charset. |
| `compression-gzip` | yes | gzip response bodies, gzip request bodies, and the zlib certificate decompressor. Pulls in `flate2`. |
| `compression-brotli` | yes | Brotli bodies and the Brotli certificate decompressor. Pulls in `brotli`. |
| `compression-deflate` | yes | deflate bodies and the zlib certificate decompressor. Pulls in `flate2`. |
| `compression-zstd` | yes | zstd bodies and the zstd certificate decompressor. Pulls in `zstd`. |
| `multipart` | yes | Adds `leyline::multipart` and `RequestBuilder::multipart`. Pulls in `stream` and `async-stream`. |
| `stream` | yes | Streaming request and response bodies. |
| `websocket` | yes | Adds `Session::websocket` and the `Ws*` types. Pulls in tungstenite. |
| `http3` | yes | Adds the `Http3` and `Race` policies, and `leyline-quiche`. |
| `socks` | no | SOCKS5 proxy tunnels. Without it, `build()` accepts a `socks5://` proxy URL and the first request fails. |
| `tower` | no | Adds `LeylineService`, a `tower_service::Service` over a session. |
| `unstable-bssl` | no | Exposes the BoringSSL `SslContextBuilder` behind `TlsContext`. The BoringSSL types are outside this crate's semver promise. |
| `bench-internals` | no | Exposes pool probes for the benchmark crate. Not for application use. |
| `full` | no | Every feature above except `bench-internals` and `unstable-bssl`. |

Each compression flag pulls in its own codec crate and gates two things: the
HTTP body path and the matching RFC 8879 certificate decompressor in the TLS
layer. Certificate compression codepoints are part of the fingerprint, so a
profile that lists an algorithm whose feature is off fails when the session
builds, with a message naming the algorithm and the feature, rather than
silently dropping the codepoint. Asking for request compression with the codec
compiled out fails the same way, from `send`.

Turn everything off and add back what you need:

```toml
[dependencies]
leyline-http = { version = "0.1", default-features = false, features = ["stream", "socks"] }
```

## BoringSSL build

`leyline-bssl-sys` builds BoringSSL from source with CMake and generates the
Rust bindings with `bindgen`. It supports six targets:

- `aarch64-apple-darwin`
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-unknown-linux-musl`
- `aarch64-unknown-linux-musl`
- `x86_64-pc-windows-msvc`

The build needs CMake 3.22 or later, a C and C++ compiler, and libclang. On
Windows it also needs the MSVC build tools and NASM. See
[Supported platforms](platforms.md).

The build uses the macOS deployment target that Rust uses
(`MACOSX_DEPLOYMENT_TARGET`, 11.0 by default) and honours `+crt-static` on
MSVC. It maps the source and output directories to `/build`, so the
libraries embed no local paths.

## Other targets

Other targets are not supported. The build stops with an error that names the
six targets.

`BORING_BSSL_PATH` links a BoringSSL that you built yourself for a supported
target. `BORING_BSSL_SOURCE_PATH` builds another BoringSSL source tree, and
`BORING_BSSL_ASSUME_PATCHED` skips the carried patches for it.
`BORING_BSSL_RUST_CPPLIB` names an extra C++ standard library to link when
your toolchain needs one.

## Symbol prefixing and openssl-sys

BoringSSL is built with `-DBORINGSSL_PREFIX=LEYLINE`, so every C export is
`LEYLINE_<name>`. The C++ structs behind `SSL` and `SSL_SESSION` are renamed
to `LEYLINE_ssl_st` and `LEYLINE_ssl_session_st`, and BoringSSL's internal
C++ code is in the `bssl::LEYLINE` namespace. The generated bindings keep the
plain Rust identifier and carry a `#[link_name]` for the prefixed export. That
lets a binary link `openssl-sys` or `boring-sys` beside Leyline.

`leyline-bssl-sys` declares `links = "leyline_bssl"`, its own key, so it does
not collide with another crate claiming `boringssl`.

The crates have no `fips`, `mlkem`, or `rpk` feature.

## MSRV

The workspace sets `rust-version = "1.96"`, and every crate sets it. The
edition is 2024; the forked `leyline-bssl*` crates keep upstream's 2021. The release gate compile-checks 1.96 and runs the test
suite on current stable. [MSRV](msrv.md) states the policy: a bump gets
its own minor release and its own changelog line, and never lands in a patch
release.

## Documentation build

`docs.rs` builds `leyline-http` for `x86_64-unknown-linux-gnu` only, with the
default feature set, because one target stays within the docs.rs build limits.
