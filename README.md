# leyline-http

[![crates.io](https://img.shields.io/crates/v/leyline-http.svg)](https://crates.io/crates/leyline-http)
[![docs.rs](https://docs.rs/leyline-http/badge.svg)](https://docs.rs/leyline-http)
[![CI](https://github.com/manaforged/leyline-http/actions/workflows/ci.yml/badge.svg)](https://github.com/manaforged/leyline-http/actions/workflows/ci.yml)
[![MIT licensed](https://img.shields.io/crates/l/leyline-http.svg)](https://github.com/manaforged/leyline-http/blob/main/LICENSE)

Leyline is a fast async HTTP/1.1, HTTP/2, and HTTP/3 client for Rust. Sessions, cookies, proxies, crawling, resumable downloads, and browser TLS profiles all come built in.

- HTTP/1.1, HTTP/2, and HTTP/3 over BoringSSL, with a thread-per-core mode (`run_per_core`)
- Sessions with shared connections, cookies, caching, and optional blocking use
- Encrypted DNS (RFC 8484 DoH) and TLS 1.3 Encrypted Client Hello (ECH) with automatic retry recovery
- mTLS (PEM and PKCS#12), certificate pinning, Unix domain sockets, interface and local-IP binding, and rotating HTTP/HTTPS/SOCKS5 proxy pools with custom proxy headers
- Browser tabs that track referers, links, and HTML forms
- Site crawling with `robots.txt`, sitemaps, rate limits, and checkpoints
- Retries, timeouts, redirects, resumable downloads, server-sent events (SSE), and WebSocket
- Optional TLS (JA3/JA4), HTTP/2, HTTP/3, and TCP (JA4T) profiles for Chrome, Firefox, Safari, Edge, Opera, and Brave

## Example

```toml
[dependencies]
leyline-http = "0.3"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

```rust,no_run
use std::time::Duration;

use leyline::{Browser, Session};

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::builder()
        .browser(Browser::Chrome158)
        .timeout(Duration::from_secs(15))
        .retry(2)
        .build()?;

    let body = session.get("https://example.com/").text().await?;
    println!("{body}");
    Ok(())
}
```

Rust 1.96 or later. Leyline supports every stable Rust released in the last six months; a minor release may raise the minimum to drop older ones. Building BoringSSL needs a C and C++ compiler and `git`.

To crawl a site while respecting `robots.txt` and per-host rate limits:

```rust,no_run
use leyline::Session;

# async fn run() -> leyline::Result<()> {
let mut crawl = Session::new().crawl("https://example.com/").limit(100);
while let Some(page) = crawl.next().await {
    let page = page?;
    println!("{} ({} bytes)", page.url(), page.text().len());
}
# Ok(())
# }
```

## Performance

HTTP/2 requests per second against a local origin on a Ryzen 9 9950X3D, with
identical request headers and verified TLS. Medians of three runs; higher is
better.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/bench-dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="docs/assets/bench-light.svg">
    <img alt="Leyline HTTP/2 throughput, browser profile speed, and client CPU per request" src="docs/assets/bench-light.svg">
  </picture>
</p>

| Client | Sequential | 8 connections, 256 in flight |
| --- | ---: | ---: |
| **Leyline** | **59,009** | **1,438,580** |
| hyper-util | 46,667 | 1,124,148 |
| reqwest | 39,015 | 891,623 |
| libcurl | 48,412 | 793,204 |
| Go net/http | 42,896 | 615,231 |

At 50,000 HTTP/2 req/s, p99 latency is 115 µs (78 µs with `run_per_core`), compared to 125 µs for libcurl, 153 µs for hyper-util, and 158 µs for reqwest. `SessionBuilder::run_per_core` runs one runtime per CPU core (sharing cookies, cache, and tokens) and reaches 921k HTTP/1.1 req/s and 28 µs open-loop HTTP/1.1 p99.

See [BENCHMARKS.md](BENCHMARKS.md) for HTTP/1.1, browser profiles, client CPU per request, confidence intervals, and reproduction commands.

## Browser profiles

A browser profile sends the TLS ClientHello, HTTP/2 and HTTP/3 settings, and
header order of a real browser build, taken from captures of that browser:

```rust,no_run
use leyline::{Browser, Session};

# async fn run() -> leyline::Result<()> {
let session = Session::builder().browser(Browser::Chrome158).build()?;
# drop(session);
# Ok(())
# }
```

Profiles send larger headers, so they cost 4% to 21% of throughput. With
profiles on, Leyline handles 1.55 times the requests per second of wreq and 5.79
times those of tls-client at 256 in flight, using 0.39× and 0.12× their client
CPU per request.

## Documentation

- [User guide](https://manaforged.github.io/leyline-http/): task-by-task guides for sessions, TLS, proxies, crawling, and throughput
- [API reference](https://docs.rs/leyline-http): rustdoc for every public type and method
- [Examples](crates/leyline/examples/): 19 runnable programs (`cargo run --example api_client`)
- [For coding agents](AGENTS.md): concise patterns and type map for coding agents
- [Upgrade guide](UPGRADE.md): 1-to-1 migration tables for breaking changes
- [Changelog](CHANGELOG.md): release notes, breaking changes, and benchmark highlights
- [Qualification](QUALIFICATION.md): what is proved, tested, and measured under failure and load

Before `1.0`, breaking changes are limited to clear correctness or ergonomic fixes, and every one is mapped in [UPGRADE.md](UPGRADE.md).

## License

[MIT](LICENSE). The BoringSSL and quiche forks keep their own licenses; see [NOTICE](NOTICE).
