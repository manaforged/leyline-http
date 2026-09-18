# Features and targets

## Cargo features

The package is `leyline-http` and the library is `leyline`. These features are
on by default: `cookies`, `charset`, `compression-gzip`, `compression-brotli`,
`compression-deflate`, `compression-zstd`, `multipart`, `stream`, `websocket`,
`http3`, and `system-trust`.

| Feature | Default | What it changes |
| --- | --- | --- |
| `cookies` | yes | Marks cookie support. The jar itself always compiles. |
| `charset` | yes | Adds `encoding_rs`. Without it, `Response::text` falls back to lossy UTF-8 instead of honoring the `Content-Type` charset. |
| `compression-gzip` | yes | gzip response bodies, gzip request bodies, and the zlib certificate decompressor. Pulls in `flate2`. |
| `compression-brotli` | yes | Brotli bodies and the Brotli certificate decompressor. Pulls in `brotli`. |
| `compression-deflate` | yes | deflate bodies and the zlib certificate decompressor. Pulls in `flate2`. |
| `compression-zstd` | yes | zstd bodies and the zstd certificate decompressor. Pulls in `zstd`. |
| `multipart` | yes | Adds `leyline::multipart` and `RequestBuilder::multipart`. Pulls in `stream` and `async-stream`. |
| `stream` | yes | Streaming request and response bodies. |
| `websocket` | yes | Adds `Session::websocket` and the `Ws*` types. Pulls in tungstenite. |
| `http3` | yes | Adds `H3Config`, the `Http3` and `Race` policies, and `leyline-quiche`. |
| `system-trust` | yes | Marks platform trust-store loading. The trust code compiles either way. |
| `socks` | no | SOCKS5 proxy tunnels. Without it, a `socks5://` proxy URL fails at connect time. |
| `native-interface-bind` | no | Marks binding a socket to a named network interface. `SocketConfig::interface` compiles either way. |
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
leyline-http = { git = "https://github.com/manaforged/leyline-http", branch = "main", default-features = false, features = ["stream", "socks"] }
```

## Prebuilt BoringSSL targets

Leyline links its own BoringSSL through `leyline-bssl-sys`, which ships
prebuilt static libraries and pregenerated bindings for four targets:

- `aarch64-apple-darwin`
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-pc-windows-msvc`

On those targets the build script only emits link directives. You need no
CMake, bindgen, Perl, or Go.

## Other targets

Other targets are not supported by the packaged crate. Adding one requires
native BoringSSL libraries, generated Rust bindings, and a matching target
configuration in `leyline-bssl-sys`.

`BORING_BSSL_PATH` overrides the native library location for an existing
target. It does not generate bindings or enable another target.

`BORING_BSSL_RUST_CPPLIB` names an extra C++ standard library to link when
your toolchain needs one.

## Symbol prefixing and openssl-sys

Shipped BoringSSL libraries are built with `-DBORINGSSL_PREFIX=LEYLINE`, so
every export is `LEYLINE_<name>`. The generated bindings keep the plain Rust
identifier and carry a `#[link_name]` for the prefixed export. Prefixing is
not optional, and there is no `prefix-symbols` feature. That is what lets a
binary link `openssl-sys` or `boring-sys` beside Leyline.

`leyline-bssl-sys` declares `links = "leyline_bssl"`, its own key, so it does
not collide with another crate claiming `boringssl`. The packaging script
fails if an unprefixed OpenSSL-style export survives. `PROVENANCE.md` records
which target artifacts have been regenerated with the prefix so far.

The `fips` and `mlkem` features are refused outright, because the packaged
artifacts do not carry those builds.

## MSRV

The workspace sets `rust-version = "1.96"`, and every crate inherits it. The
edition is 2024. The release gate compile-checks 1.96 and runs the test
suite on current stable. [docs/MSRV.md](../MSRV.md) states the policy: a bump gets
its own minor release and its own changelog line, and never lands in a patch
release.

## Documentation build

`docs.rs` builds `leyline-http` for `x86_64-unknown-linux-gnu` only, with the
default feature set, because that target has committed prebuilt libraries and
one target stays within the docs.rs build limits.
