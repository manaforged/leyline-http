# leyline-http for coding agents

Point your coding agent at this file when it writes code that uses Leyline. Leyline is an async HTTP client for Rust with sessions, crawling, resumable downloads, and optional browser impersonation. The package is `leyline-http`, and the library is `leyline`.

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
| `Session` | Send requests. Build one per account or application and clone it; clones share connections and cookies. `Session::builder()` sets `browser`, `platform`, `languages`, `hello`, and `profile`. Edge and Opera are `Browser` variants |
| `Tab` | Browse like one tab: `open` and `follow` pages, `submit` and `submit_form` forms with the page as referer |
| `RequestBuilder` | Add headers, a body, or a timeout to one request, then `.await?`, `text().await?`, `bytes().await?`, or `download_to(path).await?` |
| `Response` | Read the status and headers, then `text`, `json`, or `bytes` |
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
let page = session.get("https://example.com/").text().await?;
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
- Set session defaults on the builder with `user_agent("acme-bot/1.0")`, `header("x-api-key", key)`, and `basic_auth(user, password)`. With a `base_url`, `basic_auth` and `bearer_auth` go only to that origin.
- Call `Session::builder().before_send(|request| { ..; Ok(()) })` to add a request id or a signature: the hook runs before every attempt, retries included, and its header and body changes are sent.
- Without `.stream()`, `send` reads the whole body into memory, up to `max_body_size`, before it returns. For a large body, call `.stream()` on the request, then read chunks with `resp.bytes_stream()?` and `body.next().await`.
- Call `request.build()?` to read a request as an `http::Request<Body>`, `session.request_from(request)` to turn one back into a builder, `request.try_clone()` to send a buffered request twice, and `response.into_http()?` to hand a response to code that uses the `http` crate.
- Call `.protocol(Protocol::Http1)` on a request to force its HTTP version; the request still uses the session's connections.
- Read `resp.remote_addr()` for the address the connection was opened to and `resp.from_cache()` to learn whether the body came from the cache.
- Match on `err.kind()` or a predicate such as `err.is_retryable()`. Do not parse the error message.
- Use `session.tab()` when requests belong to a page: each request after `open` sends the current page as its `from_page`, so its headers match the kind of request.
- For the highest request rate from one process, call `Session::builder().run_per_core(|session| async move { ... })`: one single-threaded runtime per core, with cookies, cache and tokens shared and connections per core. It needs at least as many connections as cores to help.
- On a shared multi-threaded runtime with many requests in flight, set Tokio's `event_interval(4)` on the runtime builder. See `docs/guide/throughput.md`.
- Use `session.save(path)` and `Session::load(path)` to keep cookies and the identity across runs.
- Enable a bounded HTTP cache with `Session::builder().cache(bytes)`; clones share it, and `session.clear_cache()` empties it.
- Call `request.retry(n).download_to(path).await?` to survive dropped transfers: each retry resumes when the server sends a strong ETag; `sha256` and `progress` configure verification and reporting.
- Call `request.events()?` for parsed server-sent events, then await `next()`; `retry` bounds reconnections.
- Pair `bearer_auth` and `base_url` with `refresh_token` to share an asynchronous token refresh after a 401.
- Use `crawl.save(path).await?` and `Crawl::load(&session, path)?` to resume pending pages and duplicate history.
- Call `crawl.filter(|url| url.path().starts_with("/docs/"))` to follow only the links a closure accepts. A checkpoint does not save the filter, so set it again after `Crawl::load`.
- Seed a cookie with `Cookie::new(name, value)` from `leyline::cookie` and `session.cookies().insert(&url, cookie)`. `Session::builder().cookies(None)` turns cookie storage off.
- For mutual TLS with a certificate held in memory, pass `Tls::new().client_cert_pem(chain_pem, key_pem)` to `SessionBuilder::tls`.
- Pass `Proxy::custom(|url| ..)` to `proxy` to choose the proxy for each request from its URL; return `None` to send it direct.
- Test against `leyline::testing::Server` (feature `test-util`): in a handler, `request.text()` is the request body and `request.head()` the request line and headers, and `testing::Response::abort_after(n)` drops the connection after `n` body bytes.

## Read next

- [Breaking changes and upgrades](UPGRADE.md): migration steps and old-to-new API names, grouped by release
- [Changelog](CHANGELOG.md): release changes and links to migration instructions

- [API reference](https://docs.rs/leyline-http): every public item
- [User guide](https://manaforged.github.io/leyline-http/): one page per task
