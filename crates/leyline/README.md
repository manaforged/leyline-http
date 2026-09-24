# Leyline

An async HTTP client for Rust that matches the TLS ClientHello, HTTP/2
settings, and header order of a chosen browser build. Bundled profiles cover
Chrome 145 to 152, Brave 146, Firefox 148 to 154, Safari 18 and 26, Safari on
iOS 17 and 18, OkHttp on Android, and CFNetwork on iOS 18 and macOS 26.
Leyline supports HTTP/1.1, HTTP/2, HTTP/3, and WebSocket on Tokio.

## Requirements

- Rust 1.96 or later.
- Tokio.
- One of these targets. The crate ships prebuilt BoringSSL libraries and
  bindings for each:
  - `aarch64-apple-darwin`
  - `x86_64-unknown-linux-gnu`
  - `aarch64-unknown-linux-gnu`
  - `x86_64-pc-windows-msvc`

Other targets, including Intel macOS and musl, are not supported. Setting
`BORING_BSSL_PATH` does not add a target.

## Install

Add Leyline and Tokio to `Cargo.toml`:

```toml
[dependencies]
leyline-http = "0.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

The package is `leyline-http`. The library is `leyline`.

## Usage

```rust,no_run
use leyline::Session;

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::chrome();
    let mut response = session.get("https://example.com/").await?;
    println!("{}", response.text().await?);
    Ok(())
}
```

`Session::chrome()` uses the latest bundled Chrome profile with a Windows
identity. Reuse one session to share connections and cookies. To select
another browser or platform, use `Session::builder()`.

## Benchmarks

Leyline's request rate relative to each peer. A positive number means Leyline
completed more requests per second. Each cell is the mean of 20 paired rounds.
Every difference below is outside its 95% confidence interval of zero.

| Scenario | vs wreq 0.16.1 | vs reqwest 0.13.5 | vs tls-client 1.16.0 (Go) |
| --- | ---: | ---: | ---: |
| Sequential requests on one connection | +15.3% | +10.0% | -7.7% |
| New client per request (full TLS handshake) | +10.6% | -17.7% | +39.5% |
| 1 connection, 64 concurrent streams | +10.7% | -4.1% | +105.4% |
| 8 connections, 8 requests in flight | +9.8% | -1.4% | +18.1% |
| 8 connections, 256 requests in flight | +29.9% | +5.7% | +98.4% |

Leyline and wreq both use a Chrome 149 profile. Leyline and
[tls-client](https://github.com/bogdanfinn/tls-client) both use a Chrome 152
profile. reqwest uses rustls and sends no browser profile, so it skips the
work of matching a browser's TLS and HTTP/2 fingerprint. All four clients
verify the server certificate over HTTP/2 and check every response body. The
runs used one AMD Ryzen 9 9950X3D on Linux over loopback against a Hyper
server. The Rust clients were built with `cargo build --release` and
tls-client with `go build`, with no other tuning.
[BENCHMARKS.md](https://github.com/manaforged/leyline-http/blob/main/BENCHMARKS.md)
has the method, latency percentiles, confidence intervals, raw data, and the
commands to rerun them.

## Limits

- A profile covers selected TLS, HTTP/2, and header properties. It does not
  reproduce every byte a browser sends.
- The Safari 18, Safari iOS 17, and Safari iOS 18 profiles pin a JA4 value
  taken from Leyline's own output, not from a Safari capture. Safari 26 comes
  from a WKWebView capture with a synthesized Safari HTTP identity.
- The HTTP/3 QPACK decoder uses no dynamic table. This differs from
  Chromium's QPACK parameters.
- HTTP/3 through a proxy is not supported.
- `Response::audit()` values come from the configured profile and request.
  They are not packet captures.
- Detection by a remote site is not a security defect. See
  [SECURITY.md](https://github.com/manaforged/leyline-http/blob/main/SECURITY.md).

The [profile reference](https://github.com/manaforged/leyline-http/blob/main/docs/guide/profiles.md)
lists the capture status of every profile.

## Documentation

| Page | Contents |
| --- | --- |
| [API reference](https://docs.rs/leyline-http) | Rustdoc for every public type. |
| [User guide](https://github.com/manaforged/leyline-http/blob/main/docs/guide/README.md) | Task pages: sessions, requests, streaming, proxies, cookies, HTTP/3, and TLS trust. |
| [Profile reference](https://github.com/manaforged/leyline-http/blob/main/docs/guide/profiles.md) | Bundled profiles and their capture status. |
| [API map](https://github.com/manaforged/leyline-http/blob/main/docs/api.md) | Each task mapped to its type or function, and the error model. |
| [Changelog](https://github.com/manaforged/leyline-http/blob/main/CHANGELOG.md) | Release notes and the version policy. |

## Security

To report a vulnerability, follow
[SECURITY.md](https://github.com/manaforged/leyline-http/blob/main/SECURITY.md).

## Contributing

[CONTRIBUTING.md](https://github.com/manaforged/leyline-http/blob/main/CONTRIBUTING.md)
describes the build and the checks.

## License

Leyline is licensed under the
[MIT license](https://github.com/manaforged/leyline-http/blob/main/LICENSE).
[NOTICE](https://github.com/manaforged/leyline-http/blob/main/NOTICE) lists the
BoringSSL and quiche forks and their licenses.

Copyright 2026 Manaforge Technologies, LLC.
