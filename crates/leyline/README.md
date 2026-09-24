# Leyline

An async Rust HTTP client that reproduces the TLS ClientHello, HTTP/2
SETTINGS, and request header order recorded from selected browser builds. The
[profile reference](https://github.com/manaforged/leyline-http/blob/main/docs/guide/profiles.md)
states the capture status of each profile. Bundled profiles cover
Chrome 145 to 152, Brave 146, Firefox 148 to 154, Safari 18 and 26, Safari on
iOS 17 and 18, OkHttp on Android, and CFNetwork on iOS 18 and macOS 26.
Leyline supports HTTP/1.1, HTTP/2, HTTP/3, and WebSocket on Tokio.

## Requirements

- Rust 1.96 or later.
- Tokio.
- One of these targets:
  - `aarch64-apple-darwin`
  - `x86_64-unknown-linux-gnu`
  - `aarch64-unknown-linux-gnu`
  - `x86_64-pc-windows-msvc`
- The tools to build BoringSSL from source: CMake 3.22 or later, a C and C++
  compiler, and libclang for `bindgen`. On Windows, the MSVC build tools and
  NASM.

Other targets, including Intel macOS and musl, are not supported.

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

## Limits

- A profile covers selected TLS, HTTP/2, and header properties. It does not
  reproduce every byte a browser sends.
- The Safari 18, Safari iOS 17, Safari iOS 18, Firefox 148, and OkHttp
  profiles pin a JA4 value taken from Leyline's own output, not from a capture.
- Chrome 145, 146, 147, and 149 are inferred from neighbouring versions. Chrome
  151 and 152 come from `chrome-headless-shell`. Safari 26 comes from a
  WKWebView capture with a synthesized Safari HTTP identity.
- The Android identity of the Chrome profiles reuses the desktop TLS and
  HTTP/2 settings. No mobile Chrome capture exists.
- The TCP/IP fingerprint (JA4T: window size, options, MSS, TTL) comes from the
  host OS. A profile does not change it.
- BoringSSL chooses the TLS key shares. A profile sets the supported groups
  only.
- HTTP/3 has no browser capture golden. QUIC transport parameters are not
  checked against a browser. The QPACK decoder uses no dynamic table.
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
| [User guide](https://manaforged.github.io/leyline-http/) | Task pages: sessions, requests, streaming, proxies, cookies, HTTP/3, and TLS trust. |
| [API reference](https://manaforged.github.io/leyline-http/reference/leyline-http.html) | Every public item, generated from the compiler. |
| [Profile reference](https://manaforged.github.io/leyline-http/guide/profiles.html) | Bundled profiles and their capture status. |
| [API map](https://manaforged.github.io/leyline-http/api.html) | Each task mapped to its type or function, and the error model. |
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
It depends on forks of BoringSSL (MIT, Apache-2.0, and BSD-3-Clause) and
quiche (BSD-2-Clause), published as separate crates.
[NOTICE](https://github.com/manaforged/leyline-http/blob/main/NOTICE) lists
the forks and their licenses.

Copyright 2026 Manaforge Technologies, LLC.
