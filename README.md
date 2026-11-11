# leyline-http

[![crates.io](https://img.shields.io/crates/v/leyline-http.svg)](https://crates.io/crates/leyline-http)
[![docs.rs](https://docs.rs/leyline-http/badge.svg)](https://docs.rs/leyline-http)
[![CI](https://github.com/manaforged/leyline-http/actions/workflows/ci.yml/badge.svg)](https://github.com/manaforged/leyline-http/actions/workflows/ci.yml)
[![MIT licensed](https://img.shields.io/crates/l/leyline-http.svg)](https://github.com/manaforged/leyline-http/blob/main/LICENSE)

Leyline is a fast async HTTP/1.1, HTTP/2, and HTTP/3 client for Rust, with batteries included for sessions, web crawling, proxies, resumable downloads, and browser TLS profiles.

- HTTP/1.1, HTTP/2, and HTTP/3 over BoringSSL, with a thread-per-core mode (`run_per_core`)
- Sessions with shared connections, cookies, and caching
- Browser tabs that track referers, links, and HTML forms
- Site crawling with `robots.txt`, sitemaps, rate limits, and checkpoints
- Retries, timeouts, redirects, and rotating HTTP, HTTPS, and SOCKS5 proxy pools
- Resumable downloads, server-sent events (SSE), and WebSocket
- Optional TLS (JA3/JA4), HTTP/2, and HTTP/3 profiles for Chrome, Firefox, Safari, Edge, Opera, and Brave

## Example

```toml
[dependencies]
leyline-http = "0.2"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

```rust,no_run
use std::time::Duration;

use leyline::{Browser, Session};

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::builder()
        .browser(Browser::Chrome154)
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
| **Leyline** | **53,256** | **1,397,935** |
| hyper-util | 43,329 | 1,148,891 |
| reqwest | 35,733 | 861,386 |
| libcurl | 44,041 | 653,651 |
| Go net/http | 39,955 | 577,833 |

Leyline also uses less client CPU per request than each of them in all four
measured HTTP/2 workloads. At a fixed offered rate of 50,000 HTTP/2 requests per
second, its p99 latency is 124 µs, against 139 µs for hyper-util and 148 µs for
reqwest; libcurl's is lower at 88 µs. [BENCHMARKS.md](BENCHMARKS.md) has every workload, HTTP/1.1,
confidence intervals, and the commands to reproduce them.

For the highest request rate, `SessionBuilder::run_per_core` runs one runtime
per CPU core; the cores share cookies, cache and tokens and keep their own
connections. In that layout Leyline is ahead of fasthttp over
HTTP/1.1 at 8 and 64 workers and ahead of the bare h2 crate with 256 requests in
flight over eight HTTP/2 connections.

## Browser profiles

A browser profile sends the TLS ClientHello, HTTP/2 and HTTP/3 settings, and
header order of a real browser build, taken from captures of that browser:

```rust,no_run
use leyline::{Browser, Session};

# async fn run() -> leyline::Result<()> {
let session = Session::builder().browser(Browser::Chrome154).build()?;
# drop(session);
# Ok(())
# }
```

Profiles send larger headers, so they cost 5% to 21% of throughput. With
profiles on, Leyline handles 1.57 times the requests per second of wreq and 3.95
times those of tls-client at 256 in flight.

## Documentation

- [User guide](https://manaforged.github.io/leyline-http/)
- [API reference](https://docs.rs/leyline-http)
- [Upgrading from 0.1](UPGRADE.md)
- [Changelog](CHANGELOG.md)
- [Qualification](QUALIFICATION.md): what is proved, tested and measured under failure and load
- [For coding agents](AGENTS.md)

## License

[MIT](LICENSE). The BoringSSL and quiche forks keep their own licenses; see [NOTICE](NOTICE).
