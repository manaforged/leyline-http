# Leyline

[![Crates.io][crates-badge]][crates-url]
[![Docs.rs][docs-badge]][docs-url]
[![MSRV][msrv-badge]][msrv-url]
[![License][license-badge]][license-url]

Browser-profiled HTTP client for Rust with byte-level control over the TLS
ClientHello, HTTP/2 frame shape, HTTP/3 transport parameters, and TCP socket
options.

> **Status:** `2.0.0-alpha.1`. The public API will change before 1.0 — pin an
> exact version and watch [releases][releases-url].

Default Rust stacks (`reqwest` + `rustls`, `hyper` + `native-tls`) emit their
own TLS fingerprint. Leyline instead vendors a BoringSSL fork and ships a
ground-up HTTP/2 implementation so the emitted ClientHello, SETTINGS frame,
and `WINDOW_UPDATE` match the selected browser profile. You get `async` /
`tokio`, a reqwest-shaped request builder, a connection pool, a per-response
audit API returning JA3 / JA4 / JA4T / JA4H / H2 fingerprints, and a `leyline`
CLI for ad-hoc work.

## Example

Leyline runs on [tokio]. Add it to your `Cargo.toml`:

```toml
[dependencies]
leyline = "2.0.0-alpha.1"
tokio = { version = "1", features = ["full"] }
```

And then:

```rust,no_run
use leyline::{Browser, Platform, Session};
use std::time::Duration;

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::builder()
        .browser(Browser::Chrome147)
        .platform(Platform::Linux)
        .timeout(Duration::from_secs(15))
        .build()?;

    let resp = session.get("https://tls.peet.ws/api/all").send().await?;
    println!("{} {}", resp.status(), resp.audit().unwrap().ja4);
    Ok(())
}
```

More examples: [`crates/leyline/examples/`][examples-url] · [API docs][docs-url].

## Features

- **Profile-driven TLS ClientHello** — cipher and extension order, deterministic GREASE, ALPS, X25519MLKEM768 PQ key agreement.
- **Ground-up HTTP/2** — `leyline-h2` is RFC 9113, HPACK, flow control, profile-exact SETTINGS. No `hyper` / `h2` dependency.
- **HTTP/3 over [quiche]** — browser-profiled QUIC transport parameters, QPACK, H3 SETTINGS. Shares the H2 ClientHello path.
- **TCP-layer fingerprint** — TTL, MSS, window scale, DF bit, per-OS (JA4T).
- **Per-response audit** — JA3, JA4, JA4T, JA4H, H2 Akamai fingerprint on every `Response`.
- **Proxies** — HTTP `CONNECT` and SOCKS5, username/password auth.
- **Pooling** — H2 multiplexing, keepalive, TLS 1.3 session resumption.
- **Multi-language bindings** — Python, Node.js, Go wrappers over a shared BoringSSL core (C FFI).

## Supported profiles

| Profile         | Versions    | `Browser::` variants                              |
| --------------- | ----------- | ------------------------------------------------- |
| Chrome          | 145, 146, 147 | `Chrome145`, `Chrome146`, `Chrome147`           |
| Firefox         | 148         | `Firefox148`                                       |
| Safari (macOS)  | 18          | `Safari18`                                         |
| Safari (iOS)    | 15, 17, 18  | `SafariiOS15`, `SafariiOS17`, `SafariiOS18`        |
| OkHttp (Android)| 7, 10       | `OkHttpAndroid7`, `OkHttpAndroid10`                |

Adding a new version is a TOML copy-and-edit — see [`CONTRIBUTING.md`][contributing-url].

## Command-line interface

`leyline` is a fingerprint-aware HTTP client built on the library — httpie
shaped, profile-aware. Install with `cargo install --path crates/cli`:

```console
$ leyline inspect -b firefox148 https://example.com
$ leyline get https://api.example.com -H 'x-debug: 1' --json '{"q":1}'
$ leyline profile diff chrome146 chrome147
```

Verbs `get` / `post` / `put` / `patch` / `delete` / `head`; `fetch` prints
the full audit block; `inspect` dumps fingerprint / cert / wire for a `HEAD`;
`profile list|show|diff|audit` for offline introspection; `completions` emits
shell completions.

## Protocol policy

`https://` URLs go over HTTP/2 by default and fall back to HTTP/1.1 when TLS
ALPN does not negotiate H2. Force a protocol when you need to:

```rust,ignore
let h1 = Session::builder().http1().build()?;
let h2 = Session::builder().http2().build()?;
let h3 = Session::builder().http3().build()?;
let race = Session::builder().race().build()?; // sequential H3 -> H2/H1 fallback
```

Every response carries `resp.version()` and `resp.tls_alpn()` so you can see
which path actually served it.

## Testing and verification

Every wire-level claim is locked in by a test. The live suite runs against
`tls.peet.ws`, Cloudflare QUIC, and local mock proxies.

- **TLS fingerprint** — `live_ja4_exact_match_{chrome145,chrome146,chrome147,firefox148,safari18}` assert exact JA4 against `tls.peet.ws`.
- **HTTP/2 Akamai fingerprint** — `h2_fingerprints_match_toml_expectations`, `live_h2_akamai_every_profile` across all 10 profiles.
- **Post-quantum distinctness** — `chrome_pq_key_shares_use_distinct_x25519_ephemerals` guards against [utls#342] ephemeral-key reuse.
- **HTTP/3 reachability** — `live_h3_cloudflare`, `live_h3_google`, `live_h3_cloudflare_firefox_profile`.
- **JA4T per-OS** — `live_tcp_linux_ttl_is_64`, `live_tcp_windows_ttl_is_128`, `live_tcp_windows_distinguishable_from_linux`.
- **Proxy wire bytes** — `offline_http_connect_proxy_wire_bytes`, `offline_socks5_proxy_wire_bytes` pin RFC 7231 / RFC 1928 byte shapes.

```bash
cargo test --workspace
cargo test -p leyline --test tls_peet -- --ignored
cargo run -p leyline --example smoke
```

The smoke suite runs 17 end-to-end gates in one binary. Full evidence matrix and every test name: [TESTING.md][testing-url].

## Security and supply chain

- **MSRV pinned to 1.85** in the workspace `Cargo.toml`.
- **`cargo deny`** — advisories, licenses, bans, and sources gates in [`deny.toml`][deny-url]. The bans list refuses `rustls`, `openssl`, `native-tls`, `hyper`, and `h2` — anything that would route around the BoringSSL pin or the ground-up H2 impl.
- **Tracing instrumentation** — `#[tracing::instrument]` on the full request hot path, `level = "debug"`, scalar fields only. Never bodies or headers.
- **Audit-grep convention** — any method that disables a security property is prefixed `danger_`.

Report vulnerabilities through GitHub private security advisories — see
[SECURITY.md][security-url].

## Language bindings

The BoringSSL core is exposed through a C FFI and consumed by three language
wrappers, each shipping separately:

- **Python** — `pip install leyline` · [`wrappers/python`][wrappers-py]
- **Node.js** — `npm install leyline` · [`wrappers/node`][wrappers-node]
- **Go** — `go get github.com/manaforged/leyline-http/wrappers/go` · [`wrappers/go`][wrappers-go]

Each wrapper mirrors the Rust API: `leyline.get(url)` returns a response with
the same `status`, `text`, and `audit` surface.

## Architecture

```
leyline     Facade crate (use this)
  core      Session, request builder, response, WebSocket
  tls       BoringSSL connector with fingerprint control
  h2        Ground-up HTTP/2 (RFC 9113, HPACK, flow control)
  quic      HTTP/3 over QUIC (quiche + BoringSSL)
  pool      H2 connection pool
  profile   TOML browser profiles and registry
  tcp       JA4T TCP socket options via socket2
  cookies   RFC 6265 cookie jar
  audit     JA3, JA4, JA4H, JA4T computation
  ffi       C shared library for language bindings
  cli       `leyline` command-line binary
```

## Contributing

See [CONTRIBUTING.md][contributing-url]. All changes run through
`cargo test --workspace` on pre-commit and are scanned by `claim_guard` for
unsupported marketing claims. Adding a new browser profile is a TOML
copy-and-edit — the guide walks the three-step recipe.

## License

Dual-licensed under [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at
your option. Contributions are dual-licensed on the same terms per
Apache-2.0 §5.

[crates-badge]: https://img.shields.io/crates/v/leyline.svg
[crates-url]: https://crates.io/crates/leyline
[docs-badge]: https://docs.rs/leyline/badge.svg
[docs-url]: https://docs.rs/leyline
[msrv-badge]: https://img.shields.io/crates/msrv/leyline?logo=rust
[msrv-url]: https://github.com/manaforged/leyline-http/blob/main/Cargo.toml
[license-badge]: https://img.shields.io/crates/l/leyline.svg
[license-url]: https://github.com/manaforged/leyline-http#license
[releases-url]: https://github.com/manaforged/leyline-http/releases
[examples-url]: https://github.com/manaforged/leyline-http/tree/main/crates/leyline/examples
[testing-url]: TESTING.md
[security-url]: SECURITY.md
[contributing-url]: CONTRIBUTING.md
[deny-url]: deny.toml
[wrappers-py]: https://github.com/manaforged/leyline-http/tree/main/wrappers/python
[wrappers-node]: https://github.com/manaforged/leyline-http/tree/main/wrappers/node
[wrappers-go]: https://github.com/manaforged/leyline-http/tree/main/wrappers/go
[tokio]: https://tokio.rs
[quiche]: https://github.com/cloudflare/quiche
[utls#342]: https://github.com/refraction-networking/utls/issues/342
