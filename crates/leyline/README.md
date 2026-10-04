# Leyline

Leyline is an async Rust HTTP client that can mimic browsers on the wire.
The TLS ClientHello, HTTP/2 SETTINGS and priorities, HTTP/3 transport
parameters, and request header order all come from captures of shipped
browsers, so a server that fingerprints its clients sees Chrome, Firefox, or
Safari, not a Rust library.

```rust,no_run
use leyline::{Browser, Session};

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::builder()
        .browser(Browser::Chrome154)
        .audit(true)
        .build()?;

    let resp = session.get("https://example.com/").await?;
    if let Some(audit) = resp.audit() {
        // t13d1517h2_8daaf6152771_cb7bf5808d99, the JA4 of Chrome 154
        println!("{}", audit.ja4);
    }
    Ok(())
}
```

For the profiles captured from shipped browsers, the live test suite checks
that tls.peet.ws sees the same JA4 and HTTP/2 fingerprint from Leyline as
from the browser capture.

## Plain client

`Session::new()` is a plain session:

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

No browser headers. A 4xx or 5xx is not an error unless you call
`error_for_status()`. See
[API clients](https://manaforged.github.io/leyline-http/guide/api-clients.html).

## What it matches

A user agent is one header. A fingerprinting server also reads the
ClientHello, the HTTP/2 frames, and the header order. Leyline matches every
layer a capture shows:

| Layer | What Leyline sends |
| --- | --- |
| TLS | Cipher suites, extensions and their order, GREASE, key shares, and ALPN, from BoringSSL at the revision Chrome 154 ships. |
| HTTP/2 | SETTINGS, window update, pseudo-header order, and stream priority. |
| HTTP/3 | QUIC ClientHello, transport parameters, connection ID lengths, HTTP/3 SETTINGS, and the size of the first datagram. |
| Headers | The browser's header set and order for each kind of request: navigation, form, script, image, XHR, frame, and reload. |
| Fetch metadata | `sec-fetch-site`, `Origin`, and `Referer` follow the Fetch rules across redirect chains. |
| Cookies | `SameSite` follows each browser: Chrome judges the final target, Firefox the whole redirect chain. |

Of the 31 bundled profiles, 25 are captures of the shipped browser. Each
profile names the build it came from in `captured_against`, and the
[profile reference](https://github.com/manaforged/leyline-http/blob/main/docs/guide/profiles.md#provenance)
lists the source of every one.

## Profiles

| Browser | Versions | Notes |
| --- | --- | --- |
| Chrome | 145 to 154 | Android is captured for Chrome 145 only. |
| Brave | 146, 154 | |
| Firefox | 148 to 157 | Firefox 155 and 156 offer QUIC v2. |
| Safari | 18, 26, 27 | macOS. |
| Safari on iOS | 17, 18, 27 | |
| OkHttp | 4.12 | Android. |
| CFNetwork | iOS 18, iOS 27, macOS 26 | Native app traffic, not Safari. |

Edge and Opera are brand overlays on the Chrome profiles for desktop
platforms. The Opera overlay covers Chrome 145 to 152. To send a profile that
is not bundled, load it at runtime with `SessionBuilder::profile`.

## Install

Add Leyline and Tokio to `Cargo.toml`:

```toml
[dependencies]
leyline-http = "0.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

The package is `leyline-http`. The library is `leyline`.

## Requirements

- Rust 1.96 or later, and Tokio.
- One of these targets: `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`,
  `aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-musl`,
  `aarch64-unknown-linux-musl`, or `x86_64-pc-windows-msvc`. Intel macOS is
  not supported.
- The tools to build BoringSSL from source: CMake 3.22 or later, a C and C++
  compiler, and `git`. On Windows, also the MSVC build tools and NASM. On
  musl, a musl C and C++ toolchain.

The first build compiles BoringSSL, so it takes longer than a pure Rust
dependency. Later builds reuse it.

## Choose a browser

`Session::browser(Browser::default())` sends the newest bundled Chrome with a
Windows identity. A patch release can move that default to a newer capture.
To keep one fingerprint, pin the browser and the platform:

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

Reuse one session to share connections and cookies.

## Also included

- Crawling: `HostLimits`, `ProxyPool`, and `Response::block()`.
- Accounts: `Tab` for pages and forms, `Device` to save an account to a file.
- Atomic downloads and `Link` pagination.
- Proxies: HTTP and HTTPS, and SOCKS5 with the `socks` feature. The pool
  keeps connections per proxy, and HTTP/3 runs through SOCKS5
  `UDP ASSOCIATE`.
- Cookies: a jar with RFC 6265bis limits and `Secure` rules that you can save
  and restore with serde.
- WebSocket, streaming bodies, multipart forms, and gzip, deflate, Brotli,
  and zstd decoding.
- TLS trust: custom roots, certificate pins, client certificates, and a TLS
  version floor through `TlsTrustConfig`.
- Typed errors: `Error::category()` names the failure, and `is_retryable()`
  says whether to try again. Retries are off until you set a `RetryPolicy`.
- `Debug` output masks passwords, tokens, cookies, and query values.
- The bundled BoringSSL prefixes its symbols with `LEYLINE_`, so it links
  beside OpenSSL and Cloudflare's `boring`.

## Limits

- A profile covers the TLS, HTTP/2, HTTP/3, and header properties its capture
  shows. It does not reproduce every byte a browser sends, and it runs no
  JavaScript.
- Firefox on Android sends the desktop Firefox ClientHello.
- The kernel picks the TCP option order and window scale. Leyline sets TTL,
  `TCP_NODELAY`, and, where the OS allows, MSS, don't-fragment, and a window
  clamp.
- HTTP/3 sends no 0-RTT data, and it can't run through an HTTP or HTTPS
  proxy. Safari on iOS 17, OkHttp, and the CFNetwork profiles have no HTTP/3
  data.
- `Response::audit()` reports what the configured profile sends. It is not a
  packet capture.
- Detection by a remote site is not a security defect. See
  [SECURITY.md](https://github.com/manaforged/leyline-http/blob/main/SECURITY.md).

## Documentation

| Page | Contents |
| --- | --- |
| [Quick start](https://manaforged.github.io/leyline-http/guide/quick-start.html) | From an empty project to a parsed response. |
| [User guide](https://manaforged.github.io/leyline-http/) | Sessions, requests, streaming, proxies, cookies, crawling, accounts, HTTP/3, and TLS trust. |
| [API reference](https://manaforged.github.io/leyline-http/reference/leyline-http/index.html) | Every public item, and each task mapped to its one call. |
| [Profile reference](https://manaforged.github.io/leyline-http/guide/profiles.html) | Bundled profiles and their capture status. |
| [Benchmarks](https://github.com/manaforged/leyline-http/blob/main/BENCHMARKS.md) | Paired runs against wreq, reqwest, and tls-client, losses included. |
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
