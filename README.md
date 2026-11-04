# leyline-http

[![crates.io](https://img.shields.io/crates/v/leyline-http.svg)](https://crates.io/crates/leyline-http)
[![docs.rs](https://docs.rs/leyline-http/badge.svg)](https://docs.rs/leyline-http)
[![CI](https://github.com/manaforged/leyline-http/actions/workflows/ci.yml/badge.svg)](https://github.com/manaforged/leyline-http/actions/workflows/ci.yml)
[![MIT licensed](https://img.shields.io/crates/l/leyline-http.svg)](https://github.com/manaforged/leyline-http/blob/main/LICENSE)

An async Rust HTTP client for APIs, downloads, and web crawling, with optional browser impersonation.

Cloned sessions share connections, cookies, and an enabled response cache.

## Example

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

## Install

```toml
[dependencies]
leyline-http = "0.2"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Rust 1.96 or later. Building BoringSSL needs a C and C++ compiler and `git`.

## Performance

With browser profiles disabled, Leyline completed 1.37 million HTTP/2 requests per
second at 256 in flight against a local origin serving 10-byte responses on a
Ryzen 9 9950X3D. Three independent runs used certificate verification and a
30-second request timeout.

[Results, comparisons, and reproduction commands](BENCHMARKS.md#v020-measurements)
include all four workloads, CPU cost, confidence intervals, and limitations.

## Features

- HTTP/1.1, HTTP/2, and HTTP/3 over BoringSSL
- Cookies, redirects, retries, timeouts, and per-host rate limits
- HTTP, HTTPS, and SOCKS5 proxies, and proxy pools
- JSON, form, and multipart bodies; gzip, deflate, Brotli, and zstd
- WebSocket and streaming bodies
- Save and load a session with its cookies; shared asynchronous bearer-token refresh
- Resumable downloads with strong ETag checks, SHA-256 verification, and progress callbacks
- Bounded HTTP caching with conditional revalidation and credential isolation
- Parsed server-sent events with bounded reconnects and last-event-id recovery
- Site crawling with robots.txt, sitemaps, duplicate filtering, and durable checkpoints
- Optional browser profiles from real TLS, HTTP/2, HTTP/3, and header captures
- Custom root certificates, certificate pinning, and client certificates

## Documentation

- [Upgrading from 0.1 to 0.2](UPGRADE.md)

- [API reference](https://docs.rs/leyline-http)
- [User guide](https://manaforged.github.io/leyline-http/)
- [For coding agents](https://github.com/manaforged/leyline-http/blob/main/AGENTS.md)
- [Changelog](https://github.com/manaforged/leyline-http/blob/main/CHANGELOG.md)
- [Benchmarks](https://github.com/manaforged/leyline-http/blob/main/BENCHMARKS.md)

## License

[MIT](https://github.com/manaforged/leyline-http/blob/main/LICENSE). The BoringSSL and quiche forks keep their own licenses; see [NOTICE](https://github.com/manaforged/leyline-http/blob/main/NOTICE).
