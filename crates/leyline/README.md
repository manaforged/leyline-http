# Leyline

An async Rust HTTP client that sends the TLS ClientHello, HTTP/2 SETTINGS, and
request headers of a chosen browser profile. Leyline supports HTTP/1.1,
HTTP/2, HTTP/3, and WebSocket on Tokio. Bundled profiles cover Chrome 145 to
152, Brave 146, Firefox 148 to 154, Safari 18 and 26, Safari on iOS 17 and 18,
OkHttp on Android, and CFNetwork on iOS 18 and macOS 26.

## Why Leyline

- **Stated provenance for every profile.** Each profile records the build it
  was captured from in `captured_against` and its source in `capture`. The
  [profile reference](https://github.com/manaforged/leyline-http/blob/main/docs/guide/profiles.md#provenance)
  lists which profiles are browser captures, which are inferred, and which
  pin Leyline's own output.
- **A connection pool keyed by proxy.** `Session::with_proxy` switches the
  proxy and keeps the warm connections of every proxy. `Session::fresh_pool`
  takes a new pool, so the next request opens new connections.
- **Safe connection reuse.** The pool sends an HTTP/2 PING before it reuses a
  connection idle for 10 seconds, and counts failures in
  `PoolStats::h2_ping_failures`. `SocketConfig::tcp_user_timeout` bounds
  unacknowledged writes on Linux. Retries are off by default, and
  `RetryPolicy::max_retry_after` caps the wait a `Retry-After` header can ask
  for.
- **Typed errors and named timeouts.** `Error::kind` returns a `Kind` such as
  `Connect`, `Proxy`, `Tls`, or `Timeout`, and `Error::is_retryable` holds the
  one retry rule. `TimeoutConfig` has four named limits: `connect`,
  `response_header`, `read`, and `total`.
- **Prefixed BoringSSL.** The bundled BoringSSL exports its symbols with a
  `LEYLINE` prefix and declares `links = "leyline_bssl"`, so it can link into
  the same binary as `boring-sys` or `openssl-sys`. You can move one route at
  a time.
- **Chromium's `sec-ch-ua` rule.** Chromium-family profiles derive
  `sec-ch-ua` from the major version with the GREASE brand, version, and
  order rule that Chromium uses. `Response::audit()` reports the values the
  session was configured to send.
- **Profiles you load at runtime.** `SessionBuilder::profile` sends a profile
  from `ProfileRegistry::load` or `BrowserProfile::from_toml`, so a new
  browser build does not need a crate release.

## Requirements

- Rust 1.96 or later.
- Tokio.
- One of these targets:
  - `aarch64-apple-darwin`
  - `x86_64-unknown-linux-gnu`
  - `aarch64-unknown-linux-gnu`
  - `x86_64-pc-windows-msvc`
- The tools to build BoringSSL from source: CMake 3.22 or later, a C and C++
  compiler, libclang for `bindgen`, and `git`. The build script applies the
  BoringSSL patches with `git apply`. On Windows, also the MSVC build tools
  and NASM.

The first build compiles BoringSSL from source with CMake, so it takes longer
than a pure Rust dependency. Later builds reuse the compiled library.

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
    let session = Session::new();
    let body = session.get("https://example.com/").await?.text().await?;
    println!("{body}");
    Ok(())
}
```

Reuse one session to share connections and cookies. To select another
browser or platform, use `Session::builder()`.

## What `Session::new()` sends

`Session::new()` uses the newest bundled Chrome profile captured from a real
browser, with a Windows identity. It sends that profile's ClientHello,
HTTP/2 SETTINGS, user agent, `sec-ch-ua` headers, and header order. A patch
release can add a newer browser capture and move this default. To keep a
fixed fingerprint, pin the browser:

```rust,no_run
use leyline::{Browser, Platform, Session};

# fn main() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::Chrome150)
    .platform(Platform::MacOS)
    .build()?;
# drop(session);
# Ok(())
# }
```

`.platform()` selects the identity: `Windows`, `MacOS`, `Linux`, `Android`,
or `IOS`, where the profile has one. `Session::builder().build()` with no
browser is a bare session that impersonates no browser.

## Intended use

Leyline is for testing and automation of services that you are authorized to
access. Respect the terms of each site and the law that applies to you.
Leyline makes no claim that a site cannot detect it.

## Limits

- A profile covers selected TLS, HTTP/2, and header properties. It does not
  reproduce every byte a browser sends.
- The Safari 18, Safari iOS 17, Safari iOS 18, Firefox 148, and OkHttp
  profiles pin a JA4 value taken from Leyline's own output, not from a capture.
- Chrome 145, 146, 147, and 149 are inferred from neighbouring versions. Chrome
  151 and 152 come from `chrome-headless-shell`, so `Session::new()` does not
  select them. Safari 26 comes from a WKWebView capture with a synthesized
  Safari HTTP identity.
- The Android identity of the Chrome profiles reuses the desktop TLS and
  HTTP/2 settings. No mobile Chrome capture exists.
- The TCP/IP fingerprint (JA4T: window size, options, MSS, TTL) comes from the
  host OS. A profile does not change it.
- BoringSSL chooses the TLS key shares. A profile sets the supported groups
  only.
- HTTP/3 has no browser capture golden. QUIC transport parameters are not
  checked against a browser. The QPACK decoder uses no dynamic table.
- HTTP/3 through a proxy is not supported.
- A profile loaded with `SessionBuilder::profile` has no platform twin. Its
  header order comes from `header_style` in `[meta]`, and the order tables
  for each style are part of the crate.
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
