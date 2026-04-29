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
use leyline::Client;

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let client = Client::chrome()?;

    let resp = client
        .get("https://tls.peet.ws/api/all")
        .query([("view", "all")])
        .accept("application/json")
        .send()
        .await?;
    println!("{} {}", resp.status(), resp.audit().unwrap().ja4);
    Ok(())
}
```

MSRV: Rust 1.85. More examples in
[`crates/leyline/examples/`](crates/leyline/examples/).

## Quick Start For Developers

Fresh clone:

```bash
git clone https://github.com/manaforged/leyline-http
cd leyline
./scripts/dev-setup.sh
```

Windows PowerShell:

```powershell
git clone https://github.com/manaforged/leyline-http
cd leyline
.\scripts\dev-setup.ps1
```

The setup script checks Rust, native build prerequisites, then runs a small
offline build/test pass. Once that is green, use `./scripts/verify.sh --quick`
or `.\scripts\verify.ps1 -Quick` for the normal local gate and reserve live
tests for release validation.

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
| Firefox          | 148, 150, 151   |
| Safari (macOS)   | 18              |
| Safari (iOS)     | 15, 17, 18      |
| OkHttp (Android) | 7, 10           |

Adding a version is a TOML copy-and-edit — see [CONTRIBUTING.md](CONTRIBUTING.md).

## Platform support

The checked-in developer workspace is optimized for
`x86_64-pc-windows-msvc`: it patches `btls-sys` to a local prebuilt shim so a
Windows developer can build without CMake, Perl, bindgen, or libclang. Crates.io
consumers use the upstream `btls-sys` source-build path unless they opt into
their own patch.

The library code is intended to support `aarch64-apple-darwin`,
`x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, and
`aarch64-unknown-linux-gnu`, but this repository's local `btls-sys` shim only
ships Windows/MSVC artifacts today. Non-Windows source checkouts need either a
matching local BoringSSL shim bundle for their target or a workspace without the
local `[patch.crates-io] btls-sys` override.

Leyline transitively depends on `btls-sys`, which builds BoringSSL from
source on the first `cargo build`. Plan for ~5–15 minutes for that
initial build; subsequent builds are cached by Cargo.

Build dependencies, one-time per machine:

**Linux (Debian / Ubuntu):**

```bash
sudo apt-get install build-essential cmake perl pkg-config libclang-dev musl-tools git
```

**Linux (Fedora / RHEL):**

```bash
sudo dnf install gcc cmake perl pkgconf clang-devel git
```

**macOS:**

```bash
xcode-select --install
brew install cmake
```

**Windows (MSVC):** install [Visual Studio Build Tools][vsbuild] (provides
the MSVC toolchain and CMake) plus [Strawberry Perl][strawberry]
(`choco install strawberryperl`).

If you already have BoringSSL built (CI cache, prior project), set the
`BORING_BSSL_PATH` env var to that directory and `btls-sys` will skip the
source compile and just link. See the upstream [`boring2`][boring2] docs
for detail.

## What we built on

The TLS layer wraps [`0x676e67/boring2`][boring2], a BoringSSL fork that
already had the browser-shaped extensions (MLKEM768, ALPS, deterministic
GREASE, cert compression) wired up. Full attribution in [NOTICE](NOTICE).
HTTP/3 goes through [quiche] — we drive the transport parameters and H3
SETTINGS from the profile; quiche handles the rest.

The HTTP/2 implementation in [`crates/leyline/src/h2`](crates/leyline/src/h2)
is ours. Client-only, but real:
RFC 9113 §5.1 stream state machine, concurrent multiplexing, flow control
with per-stream parking, inbound RST_STREAM flood guard, classic and
extended CONNECT. Not a full server-capable stack.

Fingerprints drift. A Chrome 147 profile matches the Chrome 147 we captured;
re-verify against a fresh capture before you rely on it for anything
important.

## Wreq Parity Map

Leyline does not copy wreq's method names; it exposes the same capability
surface through browser-fidelity-first configuration.

| Capability | Leyline API |
| ---------- | ----------- |
| Typed proxies, no-proxy rules, env opt-out | `ProxyConfig`, `ProxyRule`, `NoProxy`, `SessionBuilder::proxies`, `no_proxy`, `disable_env_proxies` |
| DNS overrides and custom resolver | `DnsConfig`, `resolve_host`, `resolve_host_to_addrs`, `resolver` |
| Total/connect/read timeouts | `TimeoutConfig`, `timeout`, `connect_timeout`, `read_timeout` |
| Pool idle and max-size controls | `PoolConfig`, `pool_config`, `pool_limits`, `disable_keepalive` |
| Socket/TCP controls | `SocketConfig`, `local_address`, `tcp_nodelay`, `tcp_keepalive` |
| Redirect policy | `RedirectPolicy`, `redirect_policy`, `max_redirects` |
| Response decompression toggles | `CompressionConfig`, `compression` |
| WebSocket options | `WebSocketConfig`, `websocket_config`, `websocket_builder` |
| Tower middleware integration | `LeylineService` behind the `tower` feature |
| Feature surface | `default`, `full`, and granular flags for cookies, compression, multipart, stream, websocket, HTTP/3, Tower, SOCKS, system trust, and native interface binding |

## Testing

```bash
cargo test --workspace --exclude leyline-quiche
cargo test -p leyline --test tls_peet -- --ignored
cargo test -p leyline --test smoke -- --ignored --nocapture
```

Live tests talk to `tls.peet.ws`, Cloudflare, and Google QUIC. Evidence
matrix in [TESTING.md](TESTING.md). Perf numbers in [BENCHMARKS.md](BENCHMARKS.md).

Before tagging a release:

```bash
./scripts/dev-setup.sh           # first-clone prerequisite and fast offline check
./scripts/verify.sh              # fmt, clippy, docs, tests, deny, optional fuzz replay
.\scripts\verify.ps1 -Quick      # PowerShell quick local gate
./scripts/verify.sh --quick      # skip the live peet.ws + smoke suite
./scripts/verify.sh --fuzz 300   # run time-bounded fuzzing if fuzz/ is present
./scripts/package.sh             # package in publish order
```

## Security

Vulnerabilities go through GitHub private security advisories. Process in
[SECURITY.md](SECURITY.md). Supply-chain posture and the why of the vendored
crypto tree are in [CONTRIBUTING.md](CONTRIBUTING.md).

## Node.js

The Node.js wrapper lives in `wrappers/node` and builds a native N-API addon
from the `leyline-node` crate:

```bash
cd wrappers/node
npm run build
npm test
```

It exposes pinned browser-profile clients such as
`Client.withProfile("chrome147")` and returns Leyline audit fingerprints on
responses.

## License

Dual MIT / Apache-2.0. Third-party pieces keep their upstream licenses —
see [NOTICE](NOTICE).

[boring2]: https://github.com/0x676e67/boring2
[quiche]: https://github.com/cloudflare/quiche
[vsbuild]: https://visualstudio.microsoft.com/visual-cpp-build-tools/
[strawberry]: https://strawberryperl.com/
