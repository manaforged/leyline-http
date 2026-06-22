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
> resolve yet — leyline isn't published. A git dependency on a tier-1 target
> links leyline's checked-in prebuilt BoringSSL (zero-compile); on other targets
> it source-builds BoringSSL on first build (CMake/Perl/libclang/Go required —
> see [Platform support](#platform-support)).

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

**Cloning the repo: zero-compile on tier-1 targets.** Leyline vendors its
BoringSSL FFI as the in-repo `leyline-bssl-sys` crate, which ships prebuilt
BoringSSL artifacts, so `cargo build` links them directly — no CMake, Perl,
bindgen, libclang, or Go. This covers:

| Target | In-workspace build |
| ------ | ------------------ |
| `x86_64-pc-windows-msvc` | prebuilt — instant |
| `x86_64-unknown-linux-gnu` | prebuilt — instant |
| `aarch64-apple-darwin` (Apple Silicon) | prebuilt — instant |

Other targets — notably `x86_64-apple-darwin` (Intel Mac),
`aarch64-unknown-linux-gnu`, and musl — have **no** checked-in prebuilt yet, so
`leyline-bssl-sys`'s `build.rs` errors out (with the remedies inline). To build
for them, either point `BORING_BSSL_PATH` at a BoringSSL build for that target,
or source-build `leyline-bssl-sys` (next paragraph).

To add a target to the prebuilt set permanently, run
[`scripts/package-bssl.sh`](scripts/package-bssl.sh) on a host of that target —
it source-builds BoringSSL once and checks the static libs + bindgen output into
`crates/leyline-bssl-sys/`, then prints the two code edits needed to register the
triple (see also [`crates/leyline-bssl-sys/README.md`](crates/leyline-bssl-sys/README.md)).

**Depending on Leyline from another crate (git dependency).** External consumers
get the same checked-in prebuilt BoringSSL, so on a tier-1 target the first
`cargo build` links it directly — zero-compile. On an unsupported target,
`leyline-bssl-sys` source-builds BoringSSL once (~5–15 minutes); subsequent
builds are cached by Cargo.

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
`BORING_BSSL_PATH` env var to that directory and `leyline-bssl-sys` will skip
the source compile and just link. See the upstream [`boring2`][boring2] docs
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

## Performance

Full-stack throughput and memory against the other browser-impersonating
HTTP/TLS clients — **wreq** (Rust + BoringSSL), **azuretls** and
**bogdanfinn/tls-client** (Go + uTLS) — measured over a **real loopback TLS
socket** (not an in-process mock peer). Each client impersonates its own
newest Chrome, and the fingerprints are verified equivalent via a JA4 /
Akamai-H2 sidecar before timing. The harness runs an 18-cell matrix (`warm`/`cold` ×
`h2`/`h1` × 1 KiB/100 KiB × concurrency 1/8/64; cold is H2-only),
**5 trials per cell**, pinned cores, shuffled interleaved reps, medians +
95 % bootstrap CIs.

Each table is one test. **req/s is the median across 5 reps** at concurrency
1 / 8 / 64; RSS and CPU-s/1M are reported at the c64 peak. Full per-cell
detail (p50/p99/p99.9 latency, bootstrap CIs, every RSS/CPU point) is in
the per-run comparison tables.

### HTTP/2 (one multiplexed connection — the default path)

**warm · 1 KiB body**

| Client | req/s c1 | req/s c8 | req/s c64 | RSS MiB (c64) | CPU-s/1M (c64) |
| ------ | -------: | -------: | --------: | ------------: | -------------: |
| **leyline** | **3,093** | **16,771** | 25,396 | 11.4 | **56** |
| wreq | 2,873 | 14,223 | **27,253** | 8.8 | 71 |
| azuretls | 2,856 | 11,355 | 18,265 | 32.5 | 103 |
| bogdanfinn | 2,866 | 14,758 | 24,582 | 19.5 | 81 |

**warm · 100 KiB body**

| Client | req/s c1 | req/s c8 | req/s c64 | RSS MiB (c64) | CPU-s/1M (c64) |
| ------ | -------: | -------: | --------: | ------------: | -------------: |
| **leyline** | 1,469 | 4,103 | **5,250** | 30.5 | **226** |
| wreq | 1,317 | 4,030 | 4,344 | 19.3 | 328 |
| azuretls | 1,338 | 2,361 | 2,378 | 43.8 | 837 |
| bogdanfinn | **1,576** | **4,878** | 4,921 | 28.5 | 452 |

**cold · fresh handshake per request · 1 KiB body**

| Client | req/s c1 | req/s c8 | req/s c64 | RSS MiB (c64) | CPU-s/1M (c64) |
| ------ | -------: | -------: | --------: | ------------: | -------------: |
| **leyline** | 847 | 5,887 | 29,318 † | 11.0 | **68** |
| wreq | **987** | **6,196** | 11,876 | 14.4 | 446 |
| azuretls | 765 | 4,529 | 7,626 | 64.2 | 915 |
| bogdanfinn | 722 | 3,311 | 5,711 | 37.8 | 1,178 |

**cold · fresh handshake per request · 100 KiB body**

| Client | req/s c1 | req/s c8 | req/s c64 | RSS MiB (c64) | CPU-s/1M (c64) |
| ------ | -------: | -------: | --------: | ------------: | -------------: |
| **leyline** | 635 | 2,729 | **10,547** | 20.7 | **233** |
| wreq | **724** | **4,791** | 9,728 | 14.6 | 592 |
| azuretls | 555 | 3,544 | 5,653 | 70.9 | 1,272 |
| bogdanfinn | 573 | 2,052 | 3,551 | 54.6 | 1,570 |

leyline leads warm H2 at low/mid concurrency and on large bodies, at the
lowest CPU cost and RSS at parity with wreq (the other Rust + BoringSSL
stack); wreq edges the small-body c64 cell (CIs overlap → a tie). wreq wins
the low-concurrency cold-handshake cells. † leyline's cold c64 / 1 KiB cell
is **flagged by the harness** (cold÷warm rps = 1.15 > 0.6 ⇒ keep-alive reuse
leaked into cold mode at that concurrency); treat it as suspect, not a win.

### HTTP/1.1 (per-host connection pool)

H1 cannot multiplex, so per-host concurrency needs several connections.
leyline runs a **per-host H1 connection pool** (default cap 256) that, like
its peers, opens as many connections as concurrency demands — and now
**leads every warm H1 cell**:

**warm · 1 KiB body**

| Client | req/s c1 | req/s c8 | req/s c64 | RSS MiB (c64) | CPU-s/1M (c64) |
| ------ | -------: | -------: | --------: | ------------: | -------------: |
| **leyline** | **4,988** | **32,098** | **124,249** | 16.2 | 33 |
| wreq | 4,589 | 30,276 | 105,050 | 14.4 | **32** |
| azuretls | 3,384 | 15,459 | 19,118 | 58.2 | 365 |
| bogdanfinn | 3,641 | 15,792 | 47,151 | 48.0 | 88 |

**warm · 100 KiB body**

| Client | req/s c1 | req/s c8 | req/s c64 | RSS MiB (c64) | CPU-s/1M (c64) |
| ------ | -------: | -------: | --------: | ------------: | -------------: |
| **leyline** | **4,063** | **23,068** | **67,212** | 15.8 | **90** |
| wreq | 2,724 | 17,608 | 52,137 | 25.0 | 122 |
| azuretls | 2,647 | 9,833 | 10,367 | 67.8 | 596 |
| bogdanfinn | 2,720 | 5,830 | 10,924 | 31.3 | 411 |

For strict browser fidelity — exactly Chrome's 6 sockets per host
(`kMaxSocketsPerGroup`) — set `Session::builder().h1_max_conns_per_host(6)`.
HTTP/2 (one multiplexed connection) remains the default and primary path.

### Who wins each cell (CI-overlap = tie)

| Client | outright cell wins | tied-for-lead |
| ------ | -----------------: | ------------: |
| **leyline** | **6** | 8 |
| wreq | 3 | 7 |
| bogdanfinn | 1 | 3 |
| azuretls | 0 | — |

(One of leyline's six is the flagged cold c64 / 1 KiB cell above — five are
clean. Counts apply CI-overlap-as-tie, so they are more conservative than a
raw top-median tally.)

Numbers are loopback on WSL2 (no NIC offload, no host thermal control), so
absolute figures are machine-specific and real networks compress the
relative gaps — see the methodology notes for the fairness
controls. (The 238K req/s figure in [BENCHMARKS.md](BENCHMARKS.md) is an
in-process mock peer with no TLS or kernel socket and is *not* comparable
to the cross-client numbers above.)

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

Pick a profile (Node: `Client.chrome()` / `Client.withProfile("chrome147")`;
Python: `Client.chrome()` / `Client(profile="chrome147")`), route through a
proxy, set timeouts, or pass `audit=false`/`{ audit: false }` for the zero-cost
hot path — the surface mirrors the Rust API. Both wrappers ship **prebuilt** for
the common targets — `npm install @manaforged/leyline` (Node, via GitHub
Packages during the alpha) and a prebuilt abi3 wheel for Python — so no Rust
toolchain or BoringSSL build is needed on supported platforms; source builds
remain the fallback for other targets. Full install (registry/auth) and API
docs live in [`wrappers/node`](wrappers/node/README.md) and
[`wrappers/python`](wrappers/python/README.md).

## License

Dual MIT / Apache-2.0. Third-party pieces keep their upstream licenses -
see [NOTICE](NOTICE).

[boring2]: https://github.com/0x676e67/boring2
[quiche]: https://github.com/cloudflare/quiche
[vsbuild]: https://visualstudio.microsoft.com/visual-cpp-build-tools/
[strawberry]: https://strawberryperl.com/
