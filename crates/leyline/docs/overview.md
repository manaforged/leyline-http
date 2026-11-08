Leyline is an async HTTP client for Rust with HTTP/1.1, HTTP/2, and HTTP/3. Optional browser profiles send the TLS, HTTP/2, HTTP/3, and header fingerprints of real browsers, from captures of shipped Chrome, Firefox, and Safari builds.

Five types cover everyday requests and browsing. Configure them with setters and typed values.

| Type | Get one with | Use it to |
| --- | --- | --- |
| `Session` | `Session::new()`, `Session::from(browser)`, `Session::builder()...build()?` | Send requests, and save the session to a file. Clones share the connections and the cookies. The builder's `browser`, `platform`, `languages`, `hello`, and `profile` choose what the server sees |
| `Tab` | `session.tab()` | Open and follow pages, and submit forms with the page context |
| `RequestBuilder` | `session.get(url)`, `post`, `put`, `patch`, `delete`, `head` | Add headers, a body, or a timeout, then `.await?`, `text().await?`, `bytes().await?`, or `download_to(path).await?` |
| `Response` | `.await?` or `send().await?` | Read the status and headers, then `text`, `json`, or `bytes` |
| `Error` | any `?` | `err.kind()` says what failed |

A setter that takes a config type accepts `impl Into<Config>` where a shorthand exists, so the common case is one call:

```rust,no_run
use std::time::Duration;

use leyline::{Browser, Platform, Protocol, Session, Timeout};

# async fn run() -> leyline::Result<()> {
let session = Session::from(Browser::default());
let body = session.get("https://example.com/").text().await?;

let session = Session::builder()
    .browser(Browser::firefox()).platform(Platform::MacOS)
    .protocol(Protocol::Http2)
    .timeout(Duration::from_secs(15))
    .proxy("http://user:pass@host:8080")
    .build()?;

let resp = session
    .post("https://example.com/api")
    .header("x-trace", "1")
    .json(&serde_json::json!({ "q": 1 }))
    .timeout(Timeout::new().total(Duration::from_secs(5)))
    .send().await?;
# drop((body, resp));
# Ok(())
# }
```

## Guide

The [Leyline guide](https://manaforged.github.io/leyline-http/) covers each task with examples: sessions, requests, proxies, cookies, retries, WebSocket, HTTP/3, and testing.

## Errors

Every call returns `leyline::Result`. `Error::kind` says what failed; `Kind` lists the kinds, and [Errors](https://manaforged.github.io/leyline-http/guide/errors.html) covers retries and status mapping.

## Features

Default: `charset`, `compression-gzip`, `compression-brotli`,
`compression-deflate`, `compression-zstd`, `multipart`, `websocket`,
`http3`, `html`. Opt-in: `socks`, `tower`, `test-util`.
`full` enables every feature except `test-util`. See
[Features and targets](https://manaforged.github.io/leyline-http/guide/features-and-targets.html).

## Stability

- Every `SessionBuilder` setter that takes a config value (`Timeout`,
  `Proxy`, `Tls`) replaces it. Start from `::new()`, which
  carries the defaults. `trace` adds a hook, `resolve` adds one host, and `block_rules` adds rules
  to the ones already set. A request setter merges over the
  session value field by field.
- Public enums, and public structs with public fields, are
  `#[non_exhaustive]`, `Cookie` and `SameSite` included.
- Builder input errors surface at `build()`, `send()`, or `websocket()`,
  never eagerly, `hello` and language tags included.
- `IntoUrl` is sealed. `Session::new()` and `Session::from(..)` never
  fail: a trust-store problem fails later requests with `Kind::Tls`, and an
  invalid environment proxy fails them with `Kind::Proxy`.
- A patch release never removes a `Browser` variant. A retired profile keeps
  its variant, marked `#[deprecated]` through its `[meta] deprecated` note,
  until a minor release removes it. The newest-build functions such as
  `Browser::chrome()` and `Browser::for_platform` never return a deprecated
  profile.
- `Browser` and `Platform` serialize as the
  stable lowercase id that `id()` returns, and `FromStr` reads it back.

`SessionBuilder::profile` takes the TOML text of a profile laid out like the
bundled `profiles/<family>/<version>.toml` files, and
`Session::profile` returns it. `profiles/bare.toml` holds the bare profile and `profiles/platforms.toml`
the per-platform TCP values and the `accept_language` a profile uses when its identity table sets none. The `[meta]` keys `capture`, `variant`,
`hello`, `platform_browser`, `deprecated`, and `verified_at` are valid and
ignored at runtime, and the build reads the first five to generate
`Browser`. The TOML schema is part of the API: a patch release can add keys,
and removing or renaming one needs a minor release.
See [Profiles](https://manaforged.github.io/leyline-http/guide/profiles.html).
