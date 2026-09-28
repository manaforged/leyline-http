# Leyline

An async Rust HTTP client that sends the TLS ClientHello, HTTP/2 SETTINGS, and
request headers of a chosen browser profile. Leyline supports HTTP/1.1,
HTTP/2, HTTP/3, and WebSocket on Tokio. Bundled profiles cover Chrome 145 to
154, Brave 146 and 154, Firefox 148 to 156, Safari 18, 26, and 27, Safari on
iOS 17, 18, and 27, OkHttp on Android, and CFNetwork on iOS 18, iOS 27, and
macOS 26. Edge and Opera are brand overlays on the Chrome profiles.

## Why Leyline

- **Stated provenance for every profile.** Each profile records the build it
  was captured from in `captured_against` and its source in `capture`. The
  [profile reference](https://github.com/manaforged/leyline-http/blob/main/docs/guide/profiles.md#provenance)
  lists which profiles are browser, native stack, or emulator captures, and
  which pin Leyline's own output.
- **Captured from signed builds.** Profile values come from captures of the
  vendors' signed builds on Windows, macOS, Linux, Android, and an iPhone. The
  raw captures ship in the repository next to the profiles.
- **Headers for every request kind.** Navigation, script, XHR, form, and
  cross-origin requests each use the header order and values the browser
  sends, and redirects follow the Fetch rules for `sec-fetch-site`, `Origin`,
  and `Referer`.
- **HTTP/3 that matches the browser.** The QUIC ClientHello, transport
  parameters, connection ID lengths, HTTP/3 SETTINGS, and first datagram size
  follow each profile's capture, including QUIC v2 where the browser offers it.
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
  - `x86_64-unknown-linux-musl`
  - `aarch64-unknown-linux-musl`
  - `x86_64-pc-windows-msvc`
- The tools to build BoringSSL from source: CMake 3.22 or later, a C and C++
  compiler, libclang for `bindgen`, and `git`. The build script applies the
  BoringSSL patches with `git apply`. On Windows, also the MSVC build tools
  and NASM. On musl, a musl C and C++ cross toolchain.

The first build compiles BoringSSL from source with CMake, so it takes longer
than a pure Rust dependency. Later builds reuse the compiled library.

Other targets, including Intel macOS, are not supported.

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

`Session::new()` uses the newest captured Chrome, currently Chrome 154, with a
Windows identity. It sends that profile's ClientHello,
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

## Limits

- A profile covers the TLS, HTTP/2, HTTP/3, and header properties its capture
  shows. It does not reproduce every byte a browser sends.
- Chrome on Android is captured for Chrome 145 only. Firefox on Android sends
  the desktop Firefox ClientHello, because a profile has one TLS table for all
  platforms.
- Leyline sets TTL, MSS, and window scale where the OS allows. The TCP option
  order comes from the host OS.
- HTTP/3 does not send 0-RTT data.
- HTTP/3 through a proxy needs a SOCKS5 proxy with `UDP ASSOCIATE`. HTTP
  and HTTPS proxies cannot carry HTTP/3; MASQUE is not supported.
- A profile loaded with `SessionBuilder::profile` has no platform twin. Its
  header order comes from `header_style` in `[meta]`, and the header shapes
  in `profiles/headers.toml` are part of the crate.
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
| [API reference](https://manaforged.github.io/leyline-http/reference/leyline-http/index.html) | Every public item, generated from the compiler, and each task mapped to its one call. |
| [Profile reference](https://manaforged.github.io/leyline-http/guide/profiles.html) | Bundled profiles and their capture status. |
| [API map](https://manaforged.github.io/leyline-http/api.html) | How the API fits together, its semantics, and the error model. |
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
