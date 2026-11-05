# leyline-http

[![crates.io](https://img.shields.io/crates/v/leyline-http.svg)](https://crates.io/crates/leyline-http)
[![docs.rs](https://docs.rs/leyline-http/badge.svg)](https://docs.rs/leyline-http)
[![CI](https://github.com/manaforged/leyline-http/actions/workflows/ci.yml/badge.svg)](https://github.com/manaforged/leyline-http/actions/workflows/ci.yml)
[![MIT licensed](https://img.shields.io/crates/l/leyline-http.svg)](https://github.com/manaforged/leyline-http/blob/main/LICENSE)

A high-performance async HTTP client for Rust.

- HTTP/1.1, HTTP/2, and HTTP/3 over BoringSSL
- Sessions with shared connections, cookies, and caching
- Retries, timeouts, redirects, and per-host rate limits
- HTTP, HTTPS, and SOCKS5 proxies, with rotating pools
- Resumable downloads, server-sent events, and WebSocket
- Site crawling with robots.txt, sitemaps, and checkpoints
- Optional browser profiles for Chrome, Firefox, Safari, Edge, Opera, and Brave

Use it for API clients, web crawlers, scrapers, price trackers, stock monitors, sneaker bots, and AI agents.

## Example

```toml
[dependencies]
leyline-http = "0.2"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

```rust,no_run
use leyline::Session;

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::new();
    let body = session.get("https://example.com/").send().await?.text().await?;
    println!("{body}");
    Ok(())
}
```

Rust 1.96 or later. Building BoringSSL needs a C and C++ compiler and `git`.

## Performance

HTTP/2 requests per second against a local origin on a Ryzen 9 9950X3D, with
identical request headers and verified TLS. Medians of three runs; higher is
better.

| Client | Sequential | 8 connections, 256 in flight |
| --- | ---: | ---: |
| **Leyline** | **51,725** | **1,369,390** |
| hyper-util | 42,763 | 1,120,893 |
| reqwest | 34,921 | 854,499 |
| libcurl | 41,573 | 672,036 |
| Go net/http | 38,325 | 547,258 |

Leyline also uses less client CPU per request than each of them in all four
measured HTTP/2 workloads. [BENCHMARKS.md](BENCHMARKS.md) has every workload, HTTP/1.1,
confidence intervals, and the commands to reproduce them.

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
- [For coding agents](AGENTS.md)

## License

[MIT](LICENSE). The BoringSSL and quiche forks keep their own licenses; see [NOTICE](NOTICE).
