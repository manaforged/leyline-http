# leyline-http

[![crates.io](https://img.shields.io/crates/v/leyline-http.svg)](https://crates.io/crates/leyline-http)
[![docs.rs](https://docs.rs/leyline-http/badge.svg)](https://docs.rs/leyline-http)
[![CI](https://github.com/manaforged/leyline-http/actions/workflows/ci.yml/badge.svg)](https://github.com/manaforged/leyline-http/actions/workflows/ci.yml)
[![MIT licensed](https://img.shields.io/crates/l/leyline-http.svg)](https://github.com/manaforged/leyline-http/blob/main/LICENSE)

An async Rust HTTP client that matches the TLS, HTTP/2, and HTTP/3 fingerprints of Chrome, Edge, Opera, Brave, Firefox, and Safari.

## Features

- Browser profiles from real captures: TLS ClientHello, HTTP/2 settings and priorities, HTTP/3 transport parameters, and header order
- HTTP/1.1, HTTP/2, and HTTP/3 over BoringSSL
- Cookies, redirects, retries, timeouts, and per-host rate limits
- HTTP, HTTPS, and SOCKS5 proxies, and proxy pools
- JSON, form, and multipart bodies; gzip, deflate, Brotli, and zstd
- WebSocket and streaming bodies
- Save and load a session with its cookies
- Custom root certificates, certificate pinning, and client certificates

## Example

```rust,no_run
use leyline::{Browser, Session};

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::from(Browser::chrome());
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

## Documentation

- [API reference](https://docs.rs/leyline-http)
- [User guide](https://manaforged.github.io/leyline-http/)
- [For coding agents](https://github.com/manaforged/leyline-http/blob/main/AGENTS.md)
- [Changelog](https://github.com/manaforged/leyline-http/blob/main/CHANGELOG.md)
- [Benchmarks](https://github.com/manaforged/leyline-http/blob/main/BENCHMARKS.md)

## License

[MIT](https://github.com/manaforged/leyline-http/blob/main/LICENSE). The BoringSSL and quiche forks keep their own licenses; see [NOTICE](https://github.com/manaforged/leyline-http/blob/main/NOTICE).
