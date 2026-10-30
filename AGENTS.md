# leyline-http for coding agents

Point your coding agent at this file when it writes code that uses Leyline. Leyline is an async HTTP client for Rust that sends the TLS, HTTP/2, HTTP/3, and header fingerprints of real browsers. The package is `leyline-http`, and the library is `leyline`.

## Install

```toml
[dependencies]
leyline-http = "0.2"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

The examples that send or read JSON as Rust types also use `serde` (with `derive`) and `serde_json`.

The first build compiles BoringSSL and needs a C and C++ compiler and `git`.

## The five types

| Type | Use it to |
| --- | --- |
| `Session` | Send requests. Build one per browser and clone it; clones share connections and cookies. `Session::builder()` sets `browser`, `platform`, `languages`, `hello`, and `profile`. Edge and Opera are `Browser` variants |
| `Tab` | Browse like one tab: `open` and `follow` pages, `submit` and `submit_form` forms with the page as referer |
| `RequestBuilder` | Add headers, a body, or a timeout to one request, then `send().await?` |
| `Response` | Read the status and headers, then `text`, `json`, `bytes`, or `download_to` |
| `Error` | Match on `err.kind()` |

```rust,no_run
use std::time::Duration;

use leyline::{Browser, Session};

# async fn run() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::Chrome154)
    .timeout(Duration::from_secs(30))
    .retry(2)
    .build()?;
let page = session.get("https://example.com/").send().await?.text().await?;
session.save("session.json")?;
let again = Session::load("session.json")?.build()?;
# drop((page, again));
# Ok(())
# }
```

## Patterns that work

- Pin a browser such as `Browser::Chrome154`. `Browser::default()` moves to the newest Chrome in a patch release.
- Build one session and clone it into tasks. A new session per request opens new connections and loses cookies.
- Pass a `Duration` to `timeout`, a `u32` to `retry`, and a URL string to `proxy`; each converts into the full config type.
- Match on `err.kind()` or a predicate such as `err.is_retryable()`. Do not parse the error message.
- Use `session.tab()` when requests belong to a page: each request after `open` sends the current page as its `from_page`, so its headers match the kind of request.
- Use `session.save(path)` and `Session::load(path)` to keep cookies and the identity across runs.

## Read next

Each link is plain markdown.

- [API reference](https://docs.rs/leyline-http): every public item
- [Guides](https://raw.githubusercontent.com/manaforged/leyline-http/main/docs/SUMMARY.md): one page per task, each at `docs/guide/<name>.md`
