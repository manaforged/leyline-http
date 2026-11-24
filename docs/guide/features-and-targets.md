# Cargo features and linking

This page lists the Cargo features, how Leyline's BoringSSL coexists with
other TLS crates, and how docs.rs builds the crate. Leyline supports 6 targets
(Linux x86_64/aarch64 glibc & musl, Apple silicon macOS, Windows x86_64 MSVC);
see [Supported platforms](platforms.md).

## Cargo features

The package is `leyline-http` and the library is `leyline`. The cookie jar,
system trust-store loading, streaming bodies, SSE, and `Session::robots`
(`leyline::crawl::{Robots, RobotsSource, Purpose}`) always compile.

| Feature | Default | What it changes |
| --- | --- | --- |
| `charset` | yes | Adds `encoding_rs`. Without it, `Response::text` decodes lossy UTF-8 and ignores the `Content-Type` charset. |
| `compression-gzip` | yes | gzip bodies and the zlib certificate decompressor. Pulls in `flate2`. |
| `compression-brotli` | yes | Brotli bodies and the Brotli certificate decompressor. Pulls in `brotli`. |
| `compression-deflate` | yes | deflate bodies and the zlib certificate decompressor. Pulls in `flate2`. |
| `compression-zstd` | yes | zstd bodies and the zstd certificate decompressor. Pulls in `zstd`. |
| `multipart` | yes | Adds `leyline::multipart` and `RequestBuilder::multipart`. |
| `websocket` | yes | Adds `RequestBuilder::websocket`, `WebSocket`, and `Message`. Pulls in `tokio-tungstenite`. |
| `http3` | yes | Adds the `Http3` and `Race` policies and `leyline-quiche`. |
| `html` | yes | Implies `multipart`. Adds `leyline::html`, `Session::crawl` (`crawl::{Crawl, Page, Respect, Scope, SkipReason}`), and `Tab::submit_form`. (`crawl::Robots` always compiles.) See [Accounts](accounts.md#log-in-with-the-pages-form). |
| `socks` | no | SOCKS5 proxies, and HTTP/3 through a SOCKS5 proxy. Without it, a `socks5://` or `socks5h://` session proxy, proxy list entry, or environment proxy fails `build()` with `Kind::Config`, and one set on a request fails that request with `Kind::Proxy`. |
| `tower` | no | Implements `tower_service::Service` for `Session` (also add `tower-service = "0.3"` and `use tower_service::Service;`). See [Service integration](service-integration.md#use-the-tower-adapter). |
| `test-util` | no | Adds `leyline::testing`. Enable it in `[dev-dependencies]` only. See [Testing](testing.md). |
| `bench-internals` | no | Adds the hidden `leyline::internals` module for Leyline's own tests, benches, and fuzz targets. Outside the semver promise. |
| `full` | no | Every feature above except `test-util` and `bench-internals`. |

Each compression feature gates both the body codec and the matching RFC 8879
certificate decompressor. Certificate compression is part of a browser
profile's fingerprint, so a **browser** profile whose certificate compression
algorithm is disabled fails at `build()`. **Plain** sessions (`Session::new()`
or a builder without `.browser(...)`) work cleanly with
`default-features = false`.

For a minimal backend microservice (HTTP/1.1 + HTTP/2, gzip, and Tower, without
QUIC/WebSocket/HTML/charset):

```toml
[dependencies]
leyline-http = { version = "0.3", default-features = false, features = ["compression-gzip", "tower"] }
tower-service = "0.3"
```

Build with `RUSTFLAGS="--cfg leyline_unstable_bssl"`, or enable
`bench-internals`, to reach the BoringSSL `SslContextBuilder` behind
`TlsContext`. Both are outside the semver promise.

## Symbol prefixing and openssl-sys

Leyline links next to `openssl-sys` or `boring-sys` in one binary:

- BoringSSL is built with `-DBORINGSSL_PREFIX=LEYLINE`, so every C export
  is `LEYLINE_<name>`. The C++ structs behind `SSL` and `SSL_SESSION` are
  `LEYLINE_ssl_st` and `LEYLINE_ssl_session_st`, and the internal C++ code
  is in the `bssl::LEYLINE` namespace. The bindings keep the plain Rust
  names and carry a `#[link_name]` for the prefixed export.
- `leyline-bssl-sys` declares `links = "leyline_bssl"`, its own key, so it
  does not collide with a crate that claims `boringssl`.
- The build reads only `LEYLINE_BSSL_*` variables, so a value set for
  `boring-sys` never reaches Leyline.

The crates have no `fips`, `mlkem`, or `rpk` feature.

## Documentation build

`docs.rs` builds `leyline-http` for `x86_64-unknown-linux-gnu` only, which
keeps the build within the docs.rs limits, with `features = ["full"]`. The
docs cover every feature except `test-util` and `bench-internals`.

See [Supported platforms](platforms.md) for build targets and `LEYLINE_BSSL_*`
variables, and [MSRV](msrv.md) for the minimum Rust version.
