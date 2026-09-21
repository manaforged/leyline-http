# Leyline

HTTP client that sends the same TLS ClientHello, HTTP/2 settings, and
header order as Chrome, Firefox, or Safari. Profiles ship for Chrome 145 to
152, Brave 146, Firefox 148 to 154, Safari 18 and 26, Safari on iOS 17 and
18, OkHttp on Android, and CFNetwork on iOS and macOS. It is also the
cheapest client we have measured: about 61k CPU cycles per request against
99k for reqwest and 106k for wreq, with TLS verified on every connection.

## Requirements

Rust 1.96 or later and Tokio. Prebuilt BoringSSL libraries and Rust bindings
are included for:

- `aarch64-apple-darwin`
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-pc-windows-msvc`

Other targets require porting both the native libraries and the Rust bindings.
Setting `BORING_BSSL_PATH` alone does not add target support.

## Install

Until 0.1.0 is on crates.io, use the repository:

```toml
[dependencies]
leyline-http = { git = "https://github.com/manaforged/leyline-http", branch = "main" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

The package is `leyline-http`; the import is `leyline`. A Git dependency also
checks out the vendored BoringSSL submodule, about 500 MB. The published
packages ship prebuilt libraries and skip it.

## Usage

```rust,no_run
use leyline::Session;

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::chrome();
    let mut response = session.get("https://example.com/").await?;
    println!("{}", response.text().await?);
    Ok(())
}
```

Reuse a session to share connections and cookies. `Session::chrome()` selects
the latest bundled Chrome profile with a Windows identity. Use the builder to
select another browser or platform.

## Features

- **Self-audit.** `SessionBuilder::audit(true)` returns the JA3, JA4,
  JA4H, JA4T, and HTTP/2 fingerprints of what the session emits. No
  external capture service needed.
- **Four fingerprint layers.** TLS, HTTP/2, header order, and TCP socket
  options (JA4T).
- **HTTP/3.** On by default, with `Race` and `Http3` protocol policies.
- **Early-stop streaming.** `Response::read_until(limit, predicate)`
  decodes gzip, Brotli, zstd, and deflate while streaming and returns the
  decoded prefix, so you can stop at a marker without downloading the rest.
  A streaming request body returns an error on a 307 or 308 redirect.
- **Wire introspection.** Per-response `tls_peer_certificate`,
  `tls_cipher`, `tls_version`, `tls_alpn`, and `request_headers` show
  what was negotiated and sent.
- **Certificate pinning.** `add_pinned_leaf_sha256` in one call, no
  custom verifier.
- **Socket options.** `TCP_USER_TIMEOUT` on Linux and Android, keepalive
  tuning, source-IP binding, and Happy Eyeballs parameters.
- **Connection warming.** `preconnect` and `preconnect_via` pool a
  connection, through a chosen proxy if needed, before the request that
  matters. Idle HTTP/2 connections are PING-checked before reuse.
- **DNS pinning.** `resolve_host` and `resolve_host_to_addrs` map a host
  to chosen addresses without touching the resolver.
- **Profiles as data.** TOML profiles that you load, validate, and
  version with `ProfileRegistry`.
- **Conformance-tested.** An offline test gates each profile against
  recorded fingerprint goldens.
- **Coexists with OpenSSL.** Shipped BoringSSL is symbol-prefixed, so
  `openssl-sys` and `boring-sys` link in the same binary.
- **Structured timing.** Per-phase trace events and `Response::timing`.

## Benchmarks

Paired loopback runs on one Ryzen 9 9950X3D host against a Hyper origin
with headroom, so the client sets the rate. Every client verifies TLS and
byte-checks every response. Deltas are Leyline's throughput relative to the
peer.

| Scenario | Result |
| --- | ---: |
| 8 connections x 256 in flight vs wreq, identical request headers | +33.5% |
| 8 connections x 256 in flight vs reqwest | +20.4% |
| 1 connection x 64 streams vs wreq | +12.7% |
| Sequential keepalive vs wreq | +3.5% |
| Concurrent p50 / p99 latency | 219 / 441 µs; wreq 307 / 638, reqwest 258 / 549 |

On a 30 ms link every client converges on the round-trip floor. Expect
parity there, not these deltas. Cells where the origin was the bottleneck,
the full method, and the per-round data are in
[BENCHMARKS.md](https://github.com/manaforged/leyline-http/blob/main/BENCHMARKS.md).

## Limits

Profiles cover selected TLS, HTTP/2, and HTTP header properties. Capture
status and known gaps are listed in the
[profile reference](https://github.com/manaforged/leyline-http/blob/main/crates/leyline/docs/PROFILES.md).
Safari 26 uses a WKWebView capture with a synthesized Safari HTTP identity.

HTTP/3 proxy support is not implemented. The HTTP/3 QPACK decoder uses no
dynamic table; that differs from Chromium's QPACK parameters and is a known
protocol fingerprint difference. Opt-in `response.audit()` values describe
the configured profile and request; they are not packet captures.
Detection by a remote site is out of scope; see
[SECURITY.md](https://github.com/manaforged/leyline-http/blob/main/SECURITY.md).

## Docs

Read the [user guide](https://github.com/manaforged/leyline-http/blob/main/crates/leyline/docs/guide/README.md)
for sessions, requests, streaming, retries, proxies, TLS trust, socket
options, and fingerprints.
The [changelog](https://github.com/manaforged/leyline-http/blob/main/CHANGELOG.md)
states the version policy.

[docs/api.md](https://github.com/manaforged/leyline-http/blob/main/crates/leyline/docs/api.md)
is the API contract: a job-to-symbol map of the supported
surface, the error model, and the `#[doc(hidden)]` internals that are
off-limits.

## License

Leyline is [MIT licensed](https://github.com/manaforged/leyline-http/blob/main/LICENSE).
See [NOTICE](https://github.com/manaforged/leyline-http/blob/main/NOTICE) for the
BoringSSL and quiche forks and their licenses.
