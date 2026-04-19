# Leyline

A Rust HTTP client that sends requests that look, on the wire, like a real
browser. Chrome, Firefox, Safari, OkHttp — pick a profile, get its TLS,
HTTP/2, and TCP shape.

Status: `1.0.0-alpha.1`. API is still changing; pin exact versions.

## Example

```toml
[dependencies]
leyline = "1.0.0-alpha.1"
tokio = { version = "1", features = ["full"] }
```

```rust,no_run
use leyline::{Browser, Platform, Session};

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::builder()
        .browser(Browser::Chrome147)
        .platform(Platform::Linux)
        .build()?;

    let resp = session.get("https://tls.peet.ws/api/all").send().await?;
    println!("{} {}", resp.status(), resp.audit().unwrap().ja4);
    Ok(())
}
```

More at [`crates/leyline/examples/`](crates/leyline/examples/).

The API is shaped like `reqwest`, async on tokio. Streaming bodies, multipart
uploads, digest auth, retry with backoff, cookie jar, proxy + SOCKS5, a
`tower::Service` adapter, WebSocket (both HTTP/1.1 upgrade and RFC 8441
extended CONNECT over H2). Every response carries a per-connection audit
block — JA3, JA4, JA4T, JA4H, H2 Akamai fingerprint — so you can check what
actually went on the wire.

Concurrent requests share one H2 connection through a cloneable `H2Client`.
No per-request handshake. A stream parked on flow control doesn't block
siblings.

## Profiles

| Profile          | Versions        |
| ---------------- | --------------- |
| Chrome           | 145, 146, 147   |
| Firefox          | 148             |
| Safari (macOS)   | 18              |
| Safari (iOS)     | 15, 17, 18      |
| OkHttp (Android) | 7, 10           |

Adding a version is a TOML copy-and-edit — see [CONTRIBUTING.md](CONTRIBUTING.md).

## What we built on

The TLS layer wraps [`0x676e67/boring2`][boring2], a BoringSSL fork that
already had the browser-shaped extensions (MLKEM768, ALPS, deterministic
GREASE, cert compression) wired up. Full attribution in [NOTICE](NOTICE).
HTTP/3 goes through [quiche] — we drive the transport parameters and H3
SETTINGS from the profile; quiche handles the rest.

The HTTP/2 crate in [`crates/h2`](crates/h2) is ours. Client-only, but real:
RFC 9113 §5.1 stream state machine, concurrent multiplexing, flow control
with per-stream parking, inbound RST_STREAM flood guard, classic and
extended CONNECT. Not a full server-capable stack.

Fingerprints drift. A Chrome 147 profile matches the Chrome 147 we captured;
re-verify against a fresh capture before you rely on it for anything
important.

## Testing

```bash
cargo test --workspace
cargo test -p leyline --test tls_peet -- --ignored
cargo run -p leyline --example smoke
```

Live tests talk to `tls.peet.ws`, Cloudflare, and Google QUIC. Evidence
matrix in [TESTING.md](TESTING.md). Perf numbers in [BENCHMARKS.md](BENCHMARKS.md).

Before tagging a release:

```bash
./scripts/verify.sh              # fmt, clippy, docs, tests, deny, fuzz replay
./scripts/verify.sh --quick      # skip the live peet.ws + smoke suite
./scripts/verify.sh --fuzz 300   # add 5 min of time-bounded fuzzing per target
```

## CLI

```console
$ cargo install --path crates/cli
$ leyline get https://example.com
$ leyline inspect -b firefox148 https://example.com
$ leyline profile diff chrome146 chrome147
```

## Security

Vulnerabilities go through GitHub private security advisories. Process in
[SECURITY.md](SECURITY.md). Supply-chain posture and the why of the vendored
crypto tree are in [CONTRIBUTING.md](CONTRIBUTING.md).

## Bindings

Python, Node.js, and Go wrappers over a C FFI are in [`wrappers/`](wrappers/).
Each builds the Rust library locally; see the wrapper's README.

## License

Dual MIT / Apache-2.0. Third-party pieces keep their upstream licenses —
see [NOTICE](NOTICE).

[boring2]: https://github.com/0x676e67/boring2
[quiche]: https://github.com/cloudflare/quiche
