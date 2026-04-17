# Leyline

A Rust HTTP client that lets you pick a browser profile (Chrome, Firefox,
Safari, OkHttp) and send requests that look, on the wire, like that browser.

Status: **`2.0.0-alpha.1`**. The API will change before 1.0 — pin exact
versions.

## What it does

- Picks TLS, HTTP/2, and TCP knobs to match a chosen browser version.
- Exposes a small `reqwest`-shaped async API with streaming request and
  response bodies, multipart uploads, digest auth, and an idempotent
  retry policy.
- Multiplexes concurrent requests over a single H2 connection through
  a cloneable `H2Client` handle — no per-request TLS handshake.
- Speaks WebSocket over both HTTP/1.1 upgrade and RFC 8441 extended
  CONNECT (H2).
- Attaches a per-response audit block (JA3, JA4, JA4T, JA4H, H2
  Akamai fingerprint) so you can check what actually went on the wire.
- Has a `tower::Service` adapter for drop-in use in axum / tower stacks.
- Ships a tiny `leyline` CLI for ad-hoc requests.

This is a tool for people who already know *why* they'd want this. If you
don't need a specific browser fingerprint, use [`reqwest`].

## Example

```toml
[dependencies]
leyline = "2.0.0-alpha.1"
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

More examples: [`crates/leyline/examples/`](crates/leyline/examples/).

## Profiles

| Profile         | Versions                        |
| --------------- | ------------------------------- |
| Chrome          | 145, 146, 147                   |
| Firefox         | 148                             |
| Safari (macOS)  | 18                              |
| Safari (iOS)    | 15, 17, 18                      |
| OkHttp (Android)| 7, 10                           |

Adding a new version is a TOML copy-and-edit. See
[CONTRIBUTING.md](CONTRIBUTING.md).

## Scope and honesty

A few things worth saying out loud:

- The TLS work is a thin layer on top of [`0x676e67/boring2`][boring2],
  a BoringSSL fork with the browser-oriented extensions (MLKEM768, ALPS,
  deterministic GREASE, TLS cert compression) already wired. Credit to
  that project is load-bearing here — see [`NOTICE`](NOTICE).
- The HTTP/2 implementation in `crates/h2` is client-only but real:
  RFC 9113 §5.1 stream state machine, concurrent stream multiplexing
  over a single connection, flow control with per-stream parking,
  inbound RST_STREAM flood guard, classic + extended CONNECT. It is
  not a full RFC 9113 server-capable stack.
- HTTP/3 goes through [quiche]. QUIC transport parameters and H3 SETTINGS
  are profile-driven; the rest is quiche.
- Browser fingerprints drift. A profile pinned to Chrome 147 is the
  Chrome 147 we observed, not some Platonic ideal. Re-check against real
  captures before relying on it.

## Testing

```bash
cargo test --workspace
cargo test -p leyline --test tls_peet -- --ignored
cargo run -p leyline --example smoke
```

The live tests go out to `tls.peet.ws`, Cloudflare, and Google QUIC.
Evidence matrix: [TESTING.md](TESTING.md). Measured perf numbers
(HPACK cost, allocation footprint, multiplex throughput) are in
[BENCHMARKS.md](BENCHMARKS.md).

Before tagging a release, run the full local verify:

```bash
./scripts/verify.sh            # fmt + clippy + doc + tests + deny + benches-compile
./scripts/verify.sh --quick    # skip the live peet.ws / smoke suite
```

## CLI

```console
$ cargo install --path crates/cli
$ leyline get https://example.com
$ leyline inspect -b firefox148 https://example.com
$ leyline profile diff chrome146 chrome147
```

## Security

Report vulnerabilities through GitHub private security advisories — see
[SECURITY.md](SECURITY.md). Supply-chain posture and the rationale for the
vendored crypto tree live in [CONTRIBUTING.md](CONTRIBUTING.md).

## Language bindings

Python, Node.js, and Go wrappers over a C FFI live in
[`wrappers/`](wrappers/). They need the Rust library built locally for
now; see each wrapper's README.

## License

Dual-licensed under [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at
your option. Third-party components retain their upstream licenses — see
[NOTICE](NOTICE).

[`reqwest`]: https://crates.io/crates/reqwest
[boring2]: https://github.com/0x676e67/boring2
[quiche]: https://github.com/cloudflare/quiche
