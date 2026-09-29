# Leyline

An async Rust HTTP client that sends the TLS ClientHello, HTTP/2 SETTINGS, and
request headers of a chosen browser profile. Leyline supports HTTP/1.1,
HTTP/2, HTTP/3, and WebSocket on Tokio. Bundled profiles cover Chrome 145 to
154, Brave 146 and 154, Firefox 148 to 156, Safari 18, 26, and 27, Safari on
iOS 17, 18, and 27, OkHttp on Android, and CFNetwork on iOS 18, iOS 27, and
macOS 26. Edge and Opera are brand overlays on the Chrome profiles. They apply
to desktop platforms only, and the Opera overlay covers Chrome 145 to 152.

## What it does

- Each profile records the build it was captured from in `captured_against`
  and the kind of capture in `capture`. Of the 31 bundled profiles, 25 are
  captures of the shipped browser. The other six come from other sources:
  Safari on iOS 17 and 18 (Mobile Safari in the iOS simulator), OkHttp (a
  test app in an Android emulator), CFNetwork on iOS 18 (a test binary in the
  iOS simulator), CFNetwork on iOS 27 (Shortcuts on an iPhone), and CFNetwork
  on macOS 26 (a test binary in a macOS virtual machine). The raw captures
  are in the repository under `crates/leyline/profiles/captures/`, and the
  published crate does not include them. The
  [profile reference](https://github.com/manaforged/leyline-http/blob/main/docs/guide/profiles.md#provenance)
  lists the source of every profile.
- `RequestBuilder::preset` selects the header list for a kind of request, such
  as navigation, form submission, script, XHR, cross-origin, or same-site.
  Chromium profiles also have frame, reload, and image lists. OkHttp and
  CFNetwork profiles send one fixed list. Across a redirect chain,
  `sec-fetch-site` covers the whole chain, `Origin` becomes `null` after a hop
  to another origin, and `Referer` is set again for each hop.
- With the `http3` feature, which is on by default, the QUIC ClientHello,
  transport parameters, connection ID lengths, HTTP/3 SETTINGS, and the size
  of the first datagram come from the `[h3]` table of each profile. Firefox
  155 and 156 also list QUIC v2 as an available version. Five profiles have no
  `[h3]` table: Safari on iOS 17, OkHttp, and the three CFNetwork profiles.
  For those, `ProtocolPolicy::Http3` and `ProtocolPolicy::Race` make `build()`
  fail with `Kind::Config`.
- `Session::with_proxy` keeps the session's pool, which is keyed by proxy, so
  connections to other proxies stay warm. `Session::fresh_pool` takes a new
  pool and a new TLS session cache, so the next request opens new
  connections.
- Before it reuses an HTTP/2 connection that has been idle for 10 seconds, the
  pool sends a PING. `PoolConfig::h2_ping_after_idle` changes the delay, and
  `PoolStats::h2_ping_failures` counts the failures.
  `SocketConfig::tcp_user_timeout` bounds unacknowledged writes on Linux and
  Android.
- Retries are off by default. When a response carries a `Retry-After` value
  longer than `RetryPolicy::max_retry_after`, the retry stops and Leyline
  returns that response.
- `Error::kind` returns a `Kind` such as `Connect`, `Proxy`, `Tls`, or
  `Timeout`. `Error::is_retryable` is true for timeouts, failed connections,
  and closed connections, and for a proxy CONNECT answered with 502, 503, or
  504. `TimeoutConfig` has four limits: `connect`, `response_header`, `read`,
  and `total`.
- `Debug` output masks passwords, query values, and the values of the
  `Authorization`, `Proxy-Authorization`, `Cookie`, and `Set-Cookie` headers.
- `TlsTrustConfig` sets the trust roots, certificate pins, a client
  certificate, and a TLS version floor with `min_tls_version`.
- The bundled BoringSSL prefixes every C export with `LEYLINE_`, the
  `leyline-bssl-sys` crate declares `links = "leyline_bssl"`, and its build
  reads `LEYLINE_BSSL_*` variables. Its link name and symbols therefore do not
  collide with those of Cloudflare's `boring-sys`.
- Chromium profiles derive `sec-ch-ua` from the major version and the
  `ch_ua_brand` field of the profile, with the GREASE brand, version, and
  order rule that Chromium uses. `Response::audit()` returns `Some` when the
  session was built with `SessionBuilder::audit(true)`. It holds the JA3, JA4,
  JA4T, and JA4H fingerprints and the HTTP/2 fingerprint.
- `SessionBuilder::profile` sends a profile from `ProfileRegistry::load` or
  `BrowserProfile::from_toml`. A profile that uses an existing header style
  and needs no new BoringSSL patch works without a crate release. A new header
  style or a new BoringSSL patch still needs a crate release.

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

`Session::new()` uses the newest bundled Chrome with a Windows identity. It
sends that profile's ClientHello, HTTP/2 SETTINGS, user agent, `sec-ch-ua`
headers, and header order. With the `http3` feature it races HTTP/3 against
HTTP/2 for origins that advertised `h3` in an `Alt-Svc` header, because the
bundled Chrome profiles set `race = true` in `[h3]`. A patch release can add a
newer browser capture and move this default. To keep a fixed fingerprint, pin
the browser:

```rust,no_run
use leyline::{Browser, Platform, ProtocolPolicy, Session};

# fn main() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::Chrome150)
    .platform(Platform::MacOS)
    .protocol(ProtocolPolicy::Race)
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
- Chrome on Android is captured for Chrome 145 only. A profile has one TLS
  table for all platforms, so Firefox on Android sends the desktop Firefox
  ClientHello. Firefox 148 to 154 on Android omit
  `signed_certificate_timestamp`, so Leyline's Android identity differs from
  those versions of Firefox.
- The TCP settings come from the platform rows in `platforms.toml`, which
  exist for Windows, macOS, Linux, and iOS. Leyline sets TTL and `TCP_NODELAY`
  on every OS, don't-fragment on Linux, macOS, and Windows, MSS on Linux and
  macOS, and a window clamp on Linux. The kernel picks the TCP option order
  and the window scale. The Android identity has no row and sets no TCP
  options.
- HTTP/3 does not send 0-RTT data.
- HTTP/3 through a proxy needs the `socks` feature and a SOCKS5 proxy with
  `UDP ASSOCIATE`. HTTP and HTTPS proxies cannot carry HTTP/3; MASQUE is not
  supported.
- A bundled Safari or CFNetwork browser switches to its per-platform variant
  when you call `.platform()`. A profile loaded with `SessionBuilder::profile`
  has no per-platform variant and is used as it is. Its header order comes
  from `header_style` in `[meta]`, and the header styles in
  `profiles/headers.toml` are part of the crate.
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
