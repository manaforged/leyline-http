# leyline-http for coding agents

Point your coding agent at this file when it writes code that uses Leyline. Leyline is an async HTTP client for Rust with sessions, crawling, resumable downloads, and optional browser impersonation. The package is `leyline-http`, and the library is `leyline`.

## Install

```toml
[dependencies]
leyline-http = "0.3"
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
    .browser(Browser::Chrome158)
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

- Pin a browser such as `Browser::Chrome158`. `Browser::default()` moves to the newest Chrome in a patch release.
- Build one session and clone it into tasks. A new session per request opens new connections and loses cookies.
- Pass a `Duration` to `timeout`, a `u32` to `retry`, and a URL string to `proxy`; each converts into the full config type.
- A session does not retry until you set `retry(n)`, which allows `n` more attempts. A retry happens for what `err.is_retryable()` accepts and for a 429, 502, 503 or 504 response, on idempotent methods only; `Retry::new(n).any_method(true)` includes `POST`, `.when(|attempt| ..)` replaces the rule, and `.backoff(min, max)` sets the delay range.
- Set session defaults on the builder with `user_agent("acme-bot/1.0")`, `header("x-api-key", key)`, and `basic_auth(user, password)`. With a `base_url`, `basic_auth` and `bearer_auth` go only to that origin.
- `header` adds a value on both builders, so two calls with one name send two headers; a request header replaces the session's values of that name.
- A session `user_agent`, or a `user-agent` header on one request, also drops the browser profile's `sec-ch-ua` client hints, so the two never name different browsers.
- Send JSON with `.json(&value)` and read it with `resp.json::<T>().await?`; `serde_json::Value` works as `T`.
- Call `request.pages()` to follow `Link: rel="next"` headers as a stream of responses, or read `resp.links()` yourself.
- For code with no async runtime, call `Session::builder().build_blocking()?` and run each request with `session.block_on(session.get(url).text())?`.
- Call `Session::builder().before_send(|request| { ..; Ok(()) })` to add a request id or a signature: the hook runs before every attempt, retries included, and its changes to the method, URL, headers and body are sent; read `request.body().content_length()` for buffered, file, or sized-stream lengths, and return `Err(leyline::Error::request(cause))` to abort without retrying.
- Without `.stream()`, `send` reads the whole body into memory, up to `max_body_size`, before it returns. For a large body, call `.stream()` on the request, then read chunks with `resp.bytes_stream()?` and `body.next().await`.
- A body over `max_body_size` (100 MiB by default) fails with `Kind::BodyLimit`. Pass `None` to remove the cap on the session or on one request; `cache(None)` turns the cache off.
- Call `request.build()?` to read a request as an `http::Request<Body>`, `session.request_from(request)` to turn one back into a builder, `request.try_clone()` to send a buffered request twice, and `response.into_http()?` to hand a response to code that uses the `http` crate.
- Call `.protocol(Protocol::Http1)` on a request to force its HTTP version; the request still uses the session's connections.
- Read `resp.remote_addr()` for the address the connection was opened to and `resp.from_cache()` to learn whether the body came from the cache.
- Match on `err.kind()` against `leyline::Kind`, such as `Kind::Status`, `Kind::Timeout`, `Kind::Connect` or `Kind::BodyLimit`, or call a predicate such as `err.is_retryable()`. Do not parse the error message.
- An error status is a normal `Response` until you call `error_for_status()` on the request or the response; the error then has `Kind::Status` and `err.status()`.
- Use `session.tab()` when requests belong to a page: each request after `open` sends the current page as its `from_page`, so its headers match the kind of request.
- For the highest request rate from one process, call `Session::builder().run_per_core(|session| async move { ... })`: one single-threaded runtime per core, with cookies, cache and tokens shared and connections per core. It needs at least as many connections as cores to help.
- On a shared multi-threaded runtime with many requests in flight, set Tokio's `event_interval(4)` on the runtime builder. See `docs/guide/throughput.md`.
- Use `session.save(path)` and `Session::load(path)` to keep cookies and the identity across runs.
- Enable a bounded HTTP cache with `Session::builder().cache(bytes)`; clones share it, and `session.clear_cache()` empties it.
- Call `request.retry(n).download_to(path).await?` to survive dropped transfers: each retry resumes when the server sends a strong ETag; `sha256` and `progress` configure verification and reporting.
- Call `request.events()?` for parsed server-sent events, then await `next()`; `retry` bounds reconnections.
- Pair `bearer_auth` and `base_url` with `refresh_token` to share an asynchronous token refresh after a 401.
- Use `crawl.save(path).await?` and `crawl::Crawl::load(&session, path)?` to resume pending pages and duplicate history.
- Call `crawl.filter(|url| url.path().starts_with("/docs/"))` to follow only the links a closure accepts. A checkpoint does not save the filter, so set it again after `crawl::Crawl::load`.
- Seed a cookie with `session.cookies().set("https://example.com/", name, value)`, parse a raw header with `Cookie::parse(url, set_cookie)`, or build one with `Cookie::builder(name, value).path("/app").secure(true).build()` from `leyline::cookie` and pass it to `session.cookies().insert(url, cookie)`; `cookies_for(url)` returns matching cookies in send order, and `Session::builder().cookies(None)` turns cookie storage off.
- For mutual TLS or custom roots held in memory, pass `Tls::new().ca_pem(ca_pem).client_cert_pem(chain_pem, key_pem)` or `Tls::new().client_cert_pkcs12(der, password)` to `SessionBuilder::tls`; `Tls::danger_accept_invalid_hostnames(true)` skips only hostname checks while keeping CA and pin verification.
- For encrypted DNS and Encrypted Client Hello (ECH), call `SessionBuilder::doh("https://cloudflare-dns.com/dns-query")` paired with `Tls::new().ech(true)` (or pass raw bytes to `Tls::ech_config_list`), and check `resp.ech_accepted()`.
- Bind outgoing sockets with `SessionBuilder::local_address(ip)` (call once per address family), `SessionBuilder::interface("eth0")`, or `SessionBuilder::unix_socket(path)` (Unix, HTTP and HTTPS), and send custom proxy `CONNECT`/forward headers with `Proxy::header` or `Proxy::headers`.
- Set a custom TCP fingerprint from a JA4T string with `profile::TcpProfile::from_ja4t("64240_2-4-8-1-3_1380_10")?`, or parse a full `profile::TcpProfile` from TOML (`"ttl = 64\nmss = 1380".parse()?`) for `SessionBuilder::tcp_profile`.
- Pass `Proxy::custom(|url| ..)` to `proxy` to choose the proxy for each request from its URL; return `None` to send it direct.
- Test against `leyline::testing::Server` (enable it with `features = ["test-util"]` under `[dev-dependencies]`): `Server::http(|request| testing::Response::new(200).body("hi")).await?` starts one, and `server.url("/path")` is its absolute URL.
- In a handler, `request.method()`, `request.target()` (path and query), `request.header(name)` (any case) and `request.text()` (the body) describe the request; `request.head()` is the request line and headers as sent.
- After the calls, `server.requests().await` returns every request received, in order, without removing any. `testing::Response::delay(duration)` holds a response back and `abort_after(n)` drops the connection after `n` body bytes.

## Read next

- [Breaking changes and upgrades](UPGRADE.md): migration steps and old-to-new API names, grouped by release
- [Changelog](CHANGELOG.md): release changes and links to migration instructions

- [API reference](https://docs.rs/leyline-http): every public item
- [User guide](https://manaforged.github.io/leyline-http/): one page per task
