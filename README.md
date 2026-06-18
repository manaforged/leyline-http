# Leyline

An HTTP client for Rust. By default it's plain and honest — a
`leyline/<version>` user agent, host OS, nothing exotic on the wire — so
use it for ordinary API calls the way you'd use reqwest. Ask for a browser
and it puts that browser's exact TLS, HTTP/2, and TCP shape on the wire:
Chrome, Firefox, Safari, OkHttp. That's the path for servers that fingerprint
the client and hand a plain one a 403.

Status: `1.0.0-alpha.2`. The API still moves; pin exact versions.

## Example

Leyline is not on crates.io yet — depend on it by git until the first
published release:

```toml
[dependencies]
# No release tags exist yet — pin a specific commit for reproducible builds.
leyline = { git = "https://github.com/manaforged/leyline-http", rev = "<commit-sha>" }
tokio = { version = "1", features = ["full"] }
```

> **Heads up:** a plain `leyline = "1.0.0-alpha.2"` (crates.io) does **not**
> resolve yet. A git dependency pulls the upstream `btls-sys` source-build
> path, which compiles BoringSSL on first build (CMake/Perl/libclang/Go
> required — see [Platform support](#platform-support)). For the zero-compile
> prebuilt experience, clone the repo and build inside the workspace.

```rust,no_run
use leyline::Session;

#[tokio::main]
async fn main() -> leyline::Result<()> {
    // Bare by default — a plain client, no browser fingerprint.
    let resp = leyline::get("https://api.example.com/v1").await?;
    println!("{}", resp.status());

    // Ask for Chrome when you need to look like a browser.
    let chrome = Session::chrome();
    let page = chrome.navigate("https://example.com").await?;
    println!("{}", page.status());
    Ok(())
}
```

Auditing what actually went on the wire is opt-in — turn it on and the
response carries the fingerprint Leyline sent:

```rust,no_run
use leyline::Session;

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::builder().chrome().audit(true).build()?;
    let resp = session.navigate("https://tls.peet.ws/api/all").await?;
    if let Some(audit) = resp.audit() {
        println!("JA4: {}", audit.ja4);
    }
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

The API is Leyline-native and async on tokio. Streaming bodies, multipart
uploads, digest auth, retry with backoff, cookie jar, proxy + SOCKS5, a
`tower::Service` adapter, WebSocket (both HTTP/1.1 upgrade and RFC 8441
extended CONNECT over H2). Turn on auditing (`SessionBuilder::audit(true)`)
and each response carries a per-connection audit block - JA3, JA4, JA4T,
JA4H, H2 Akamai fingerprint - so you can check what actually went on the
wire.

Concurrent requests share one H2 connection through a cloneable `H2Client`.
No per-request handshake. A stream parked on flow control doesn't block
siblings.

## Profiles

| Profile          | Versions             |
| ---------------- | -------------------- |
| Chrome           | 145, 146, 147, 148   |
| Aloha            | 138             |
| Brave            | 146             |
| Firefox          | 148, 150, 151   |
| Safari (macOS)   | 18              |
| Safari (iOS)     | 15, 17, 18      |
| OkHttp (Android) | 7, 10           |

Adding a version is a TOML copy-and-edit - see [CONTRIBUTING.md](CONTRIBUTING.md).

## Platform support

**Cloning the repo: zero-compile on tier-1 targets.** The workspace patches
`btls-sys` to a local shim that ships prebuilt BoringSSL artifacts, so
`cargo build` links them directly — no CMake, Perl, bindgen, libclang, or Go.
This covers:

| Target | In-workspace build |
| ------ | ------------------ |
| `x86_64-pc-windows-msvc` | prebuilt — instant |
| `x86_64-unknown-linux-gnu` | prebuilt — instant |
| `aarch64-apple-darwin` (Apple Silicon) | prebuilt — instant |

Other targets — notably `x86_64-apple-darwin` (Intel Mac),
`aarch64-unknown-linux-gnu`, and musl — have **no** checked-in prebuilt yet, so
the local shim's `build.rs` errors out (with the remedies inline). To build for
them, either point `BORING_BSSL_PATH` at a BoringSSL build for that target, or
remove the `[patch.crates-io] btls-sys` line from the root `Cargo.toml` to fall
back to the upstream `btls-sys` source build (next paragraph).

To add a target to the prebuilt set permanently, run
[`scripts/package-bssl.sh`](scripts/package-bssl.sh) on a host of that target —
it source-builds upstream BoringSSL once and checks the static libs + bindgen
output into `crates/btls-sys/`, then prints the two code edits needed to
register the triple (see also [`crates/btls-sys/README.md`](crates/btls-sys/README.md)).

**Depending on Leyline from another crate (git dependency).** External
consumers don't inherit the local shim, so they use the upstream `btls-sys`,
which builds BoringSSL from source on the first `cargo build`. Plan for
~5–15 minutes for that initial build; subsequent builds are cached by Cargo.

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
HTTP/3 goes through [quiche] - we drive the transport parameters and H3
SETTINGS from the profile; quiche handles the rest.

The HTTP/2 implementation in [`crates/leyline/src/h2`](crates/leyline/src/h2)
is ours. Client-only, but real:
RFC 9113 section 5.1 stream state machine, concurrent multiplexing, flow control
with per-stream parking, inbound RST_STREAM flood guard, classic and
extended CONNECT. Not a full server-capable stack.

Fingerprints drift. A Chrome 147 profile matches the Chrome 147 we captured;
re-verify against a fresh capture before you rely on it for anything
important.

## Capability Map

Leyline's API is organized around browser-fidelity-first configuration and a
small set of explicit policy objects.

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
| Feature surface | `default`, `full`, and granular flags for cookies, compression, multipart, stream, websocket, HTTP/3, Tower, SOCKS, system trust, and native interface binding. H3, WebSocket, multipart, and SOCKS gate their heavy transport surface; Brotli remains in the minimal graph for TLS certificate compression. |

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

## Language bindings

Leyline ships first-class **Node.js** and **Python** wrappers over the same Rust
core, so the browser-fidelity fingerprints and audit data are identical in all
three languages. Each wrapper feels like the HTTP client you already reach for.

### Node.js

```js
const { Client } = require('@manaforged/leyline');

const client = Client.chrome();                  // real Chrome TLS + HTTP/2
const resp = await client.get('https://api.example.com/v1');

resp.ok;                 // true
resp.json();             // parsed JSON body
resp.header('x-id');     // case-insensitive header lookup
resp.audit.ja4;          // the fingerprint we actually sent

// POST JSON — serialized and content-typed for you
await client.post('https://api.example.com/items', { json: { name: 'ada' } });
```

### Python

```python
from leyline import Client

client = Client.chrome()                         # real Chrome TLS + HTTP/2
resp = client.get("https://api.example.com/v1")

resp.ok                  # True
resp.json()              # parsed JSON body
resp.header("x-id")      # case-insensitive header lookup
resp.audit.ja4           # the fingerprint we actually sent

# POST JSON
client.post("https://api.example.com/items", json={"name": "ada"})
```

`asyncio` works too — swap `Client` for `AsyncClient` and `await` the calls.

Pick a profile (`Client.chrome()`, `Client.firefox()`, `Client.withProfile("chrome147")`),
route through a proxy, set timeouts, or pass `audit=false`/`{ audit: false }` for
the zero-cost hot path — the surface mirrors the Rust API. Both wrappers are
consumed from source by git rev (no npm/PyPI registry yet); full install and API
docs live in [`wrappers/node`](wrappers/node/README.md) and
[`wrappers/python`](wrappers/python/README.md).

## License

Dual MIT / Apache-2.0. Third-party pieces keep their upstream licenses -
see [NOTICE](NOTICE).

[boring2]: https://github.com/0x676e67/boring2
[quiche]: https://github.com/cloudflare/quiche
[vsbuild]: https://visualstudio.microsoft.com/visual-cpp-build-tools/
[strawberry]: https://strawberryperl.com/
