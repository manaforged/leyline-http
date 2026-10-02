# Leyline

Leyline is an async Rust HTTP client. A plain session is a client for APIs
and services: a base URL, a token, timeouts, retries, and JSON. A browser
session connects the way a real browser does, with the TLS, HTTP/2, HTTP/3,
and header fingerprint of a captured Chrome, Firefox, or Safari.

```rust,no_run
use std::time::Duration;

use leyline::{RetryPolicy, Session};

# async fn run() -> leyline::Result<()> {
let page = leyline::get("https://example.com/").await?.text().await?;

let api = Session::builder()
    .base_url("https://api.example/v1/")
    .bearer_auth("my-token")
    .user_agent("my-tool/1.0")
    .timeout(Duration::from_secs(30))
    .retry(RetryPolicy::transient())
    .build()?;
let status = api.get("status").error_for_status().await?;
# drop((page, status));
# Ok(())
# }
```

A 4xx or 5xx status is a response, not an error. `error_for_status()` turns
it into a `Kind::Status` error that keeps the status, the headers, and the
start of the body. A plain session sends `user-agent: leyline/<version>`,
`accept: */*`, and `accept-encoding`, and no `sec-*` headers or client hints.
[API clients](https://manaforged.github.io/leyline-http/guide/api-clients.html)
builds a complete client.

## Look like a browser

A browser session sends the ClientHello, HTTP/2 SETTINGS and priorities,
HTTP/3 transport parameters, and header order of a shipped browser, so a
server that fingerprints its clients sees the browser, not a Rust library.

```rust,no_run
use leyline::{Browser, Platform, Session};

# async fn run() -> leyline::Result<()> {
let chrome = Session::browser(Browser::default());
let pinned = Session::builder()
    .browser(Browser::Chrome154)
    .platform(Platform::MacOS)
    .audit(true)
    .build()?;
let resp = pinned.get("https://shop.example/").await?;
if let Some(audit) = resp.audit() {
    println!("{}", audit.ja4);
}
# drop(chrome);
# Ok(())
# }
```

`Browser::default()` is the newest bundled Chrome on Windows, and a patch
release can move it to a newer capture. Pin the browser and the platform to
keep one fingerprint. `audit(true)` reports the JA3, JA4, and HTTP/2
fingerprints a session presents; the live test suite checks that tls.peet.ws
reports the same JA4 and HTTP/2 fingerprint for Leyline as for each captured
browser.

| Layer | What Leyline sends |
| --- | --- |
| TLS | Cipher suites, extensions and their order, GREASE, key shares, and ALPN, from BoringSSL at the revision Chrome 154 ships |
| HTTP/2 | SETTINGS, window update, pseudo-header order, and stream priority |
| HTTP/3 | QUIC ClientHello, transport parameters, connection ID lengths, HTTP/3 SETTINGS, and the size of the first datagram |
| Headers | The browser's header set and order for navigation, form, script, image, XHR, frame, and reload requests |
| Fetch metadata | `sec-fetch-site`, `Origin`, and `Referer` follow the Fetch rules across redirect chains |
| Cookies | `SameSite` follows each browser: Chrome judges the final target, Firefox the whole redirect chain |

### Profiles

| Browser | Versions | Notes |
| --- | --- | --- |
| Chrome | 145 to 154 | Android is captured for Chrome 145 only |
| Brave | 146, 154 | |
| Firefox | 148 to 156 | Firefox 155 and 156 offer QUIC v2 |
| Safari | 18, 26, 27 | macOS |
| Safari on iOS | 17, 18, 27 | |
| OkHttp | 4.12 | Android |
| CFNetwork | iOS 18, iOS 27, macOS 26 | Native app traffic, not Safari |

Of the 31 bundled profiles, 25 are captures of the shipped browser; the
[profile reference](https://github.com/manaforged/leyline-http/blob/main/docs/guide/profiles.md#provenance)
lists the source of each. Edge and Opera are brand overlays on the desktop
Chrome profiles; Opera covers Chrome 145 to 152. `SessionBuilder::profile`
loads a profile that is not bundled.

## Features

- Streaming bodies, atomic file downloads, multipart uploads, `Link`
  pagination, and gzip, deflate, Brotli, and zstd decoding.
- Typed errors: `Error::category()` names the failure, and `is_retryable()`
  says whether to try again. Retries are off until you set a `RetryPolicy`.
- `Tab` sends each request with the referer and fetch metadata of the current
  page, and submits HTML forms (`html` feature, on by default). `Device`
  saves an account's browser, proxy, cookies, and TLS state in one file.
- `HostLimits` caps requests in flight and the rate per host. `ProxyPool`
  keeps an origin on one proxy and bans proxies that fail.
- HTTP and HTTPS proxies, and SOCKS5 with the `socks` feature, off by
  default. HTTP/3 runs through SOCKS5 `UDP ASSOCIATE`.
- A cookie jar with RFC 6265bis limits that you can save with serde.
- WebSocket, a Tower service, custom TLS roots, certificate pins, and client
  certificates.
- `Debug` output masks passwords, tokens, cookies, and query values.
- The bundled BoringSSL prefixes its symbols with `LEYLINE_`, so it links
  beside OpenSSL and Cloudflare's `boring`.

## Install

```toml
[dependencies]
leyline-http = "0.1"
tokio = { version = "1", features = ["full"] }
```

The package is `leyline-http`. The library is `leyline`.

Leyline needs Rust 1.96 or later and Tokio, on one of these targets:
`aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`,
`aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-musl`,
`aarch64-unknown-linux-musl`, or `x86_64-pc-windows-msvc`. Intel macOS is not
supported. The build links prebuilt BoringSSL with pregenerated bindings and
needs no CMake, C compiler, or libclang.
[Supported platforms](https://manaforged.github.io/leyline-http/guide/platforms.html)
describes the source build.

## Limits

- A profile covers the TLS, HTTP/2, HTTP/3, and header properties its capture
  shows. It does not reproduce every byte a browser sends, and it runs no
  JavaScript.
- Firefox on Android sends the desktop Firefox ClientHello.
- The kernel picks the TCP option order and window scale. Leyline sets TTL,
  `TCP_NODELAY`, and, where the OS allows, MSS, don't-fragment, and a window
  clamp.
- HTTP/3 sends no 0-RTT data and cannot run through an HTTP or HTTPS proxy.
  Safari on iOS 17, OkHttp, and the CFNetwork profiles have no HTTP/3 data.
- `Response::audit()` reports what the configured profile sends. It is not a
  packet capture.

## Documentation

| Page | Contents |
| --- | --- |
| [Quick start](https://manaforged.github.io/leyline-http/guide/quick-start.html) | From an empty project to a parsed response |
| [User guide](https://manaforged.github.io/leyline-http/) | Sessions, requests, streaming, proxies, cookies, crawling, accounts, HTTP/3, and TLS trust |
| [API reference](https://manaforged.github.io/leyline-http/reference/leyline-http/index.html) | Every public item |
| [Profile reference](https://manaforged.github.io/leyline-http/guide/profiles.html) | Bundled profiles and their capture status |
| [Benchmarks](https://github.com/manaforged/leyline-http/blob/main/BENCHMARKS.md) | Paired runs against wreq, reqwest, and tls-client, losses included |
| [Changelog](https://github.com/manaforged/leyline-http/blob/main/CHANGELOG.md) | Release notes and the version policy |

## Security

To report a vulnerability, follow
[SECURITY.md](https://github.com/manaforged/leyline-http/blob/main/SECURITY.md).
Detection by a remote site is not a security defect.

## Contributing

Leyline does not accept external pull requests until its API is more stable.
To report a bug or request a feature, open an
[issue](https://github.com/manaforged/leyline-http/issues).
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
