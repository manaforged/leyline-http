# Quick start

This chapter takes you from an empty project to a request and a parsed
response. Each section links to the chapter that owns the topic.

## Add the dependency

The package is `leyline-http`. The library it builds is `leyline`, so that is
the name you import. Leyline is async and runs on Tokio.

```toml
[dependencies]
leyline-http = "0.3"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

`serde` and `serde_json` are only for the JSON examples. The first build
compiles BoringSSL, so it needs a C and C++ compiler and Git. [Supported platforms](platforms.md) lists the targets and the tools.

## Send a GET request

`leyline::get` sends one GET from a temporary plain session. For more than one
request, build a `Session` once and reuse it so requests share the connection
pool and cookie jar.

```rust,no_run
#[tokio::main]
async fn main() -> leyline::Result<()> {
    let body = leyline::get("https://example.com/").text().await?;
    println!("{} bytes", body.len());

    let session = leyline::Session::new();
    let resp = session.get("https://example.com/").await?;
    println!("{} {}", resp.status(), resp.url());
    Ok(())
}
```

`Session::get` and its siblings return a `RequestBuilder`. Awaiting the
builder sends the request and returns a `Response`; `.text().await?` and
`.bytes().await?` send the request and read the body in one step. `.send()`
does the same as awaiting the builder and returns a named future.

## Choose a plain or a browser session

| Session | Build it with | Sends |
| --- | --- | --- |
| Plain | `Session::new()` or `Session::builder().build()` | `user-agent: leyline/<version>`, `accept: */*`, and no `sec-*` headers |
| Browser | `Session::from(Browser::default())` | The TLS, HTTP/2, HTTP/3, and headers of the newest bundled Chrome on Windows |
| Pinned browser | `Session::builder().browser(Browser::Chrome158).build()` | One fixed browser profile |

Use a plain session for APIs. Use a browser session when the server checks
that the client is a browser. `SessionBuilder::user_agent` replaces the
default `user-agent` on either kind.

```rust,no_run
use leyline::{Browser, Session};

# fn run() -> leyline::Result<()> {
let plain = Session::builder().user_agent("my-app/1.0").build()?;
let chrome = Session::from(Browser::default());
let pinned = Session::builder().browser(Browser::Chrome158).build()?;
# drop((plain, chrome, pinned));
# Ok(())
# }
```

[Sessions](sessions.md#plain-and-browser-sessions) lists the exact headers each kind sends.

## Read the body

`text()` decodes the body with the charset from `Content-Type`, falling back to
UTF-8. `json::<T>()` deserializes into any `DeserializeOwned` type and fails
with `Kind::Json` on invalid JSON. Each body call consumes the response, so read
`status()` and `headers()` first.

```rust,no_run
use serde::Deserialize;

#[derive(Deserialize)]
struct State {
    name: String,
    ready: bool,
}

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let page = session.get("https://example.com/").text().await?;
let state = session
    .get("https://api.example/state")
    .send().await?
    .json::<State>()
    .await?;
println!("{} {} {}", page.len(), state.name, state.ready);
# Ok(())
# }
```

## Send JSON, a form, and query parameters

| Method | Effect |
| --- | --- |
| `json(&value)` | Serializes as JSON and sets `content-type: application/json` |
| `form(pairs)` | Sends `application/x-www-form-urlencoded` from any iterator of `(AsRef<str>, AsRef<str>)` |
| `query(pairs)` | Appends query pairs to the URL |
| `timeout(Duration)` | Sets `Timeout::total` across retries; use `Timeout::new().body(..)` for streamed bodies and downloads |

```rust,no_run
use std::time::Duration;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let created = session
    .post("https://api.example/items")
    .json(&serde_json::json!({ "name": "leyline" }))
    .timeout(Duration::from_secs(10))
    .send().await?;
let found = session
    .get("https://api.example/search")
    .query([("q", "leyline"), ("page", "2")])
    .send().await?;
let login = session
    .post("https://api.example/login")
    .form([("user", "ada"), ("password", "secret")])
    .send().await?;
println!("{} {} {}", created.status(), found.status(), login.status());
# Ok(())
# }
```

See [Requests](requests.md) and
[Retries and timeouts](retries-and-timeouts.md).

## Handle a 404

A 4xx or 5xx response is `Ok` by default. Call `.error_for_status()` on the
request builder to get a status of 400 or more as a `Kind::Status` error. The
error keeps the status, the headers, and up to 64 KiB of the body.
`leyline::http` re-exports the `http` crate, so `StatusCode` needs no extra
dependency.

```rust,no_run
use leyline::http::StatusCode;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
match session.get("https://api.example/users/7").error_for_status().send().await {
    Ok(resp) => println!("{}", resp.text().await?),
    Err(err) if err.status() == Some(StatusCode::NOT_FOUND) => {
        println!("no such user: {}", err.body().map(String::from_utf8_lossy).unwrap_or_default());
    }
    Err(err) => return Err(err),
}
# Ok(())
# }
```

`Error::kind()` sorts every failure into one `Kind`, such as
`Timeout`, `Dns`, `Tls`, or `Status`, for a short message. See
[Errors](errors.md).

## Use `?` with other error types

`leyline::Error` converts from `std::io::Error` and `url::ParseError`, so `?`
works on file calls in a function that returns `leyline::Result`. It
implements `std::error::Error + Send + Sync + 'static`, so `?` also converts it
into `Box<dyn std::error::Error + Send + Sync>`, `anyhow::Error`, or your own
error type.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let token = std::fs::read_to_string("token.txt")?;
let session = leyline::Session::builder()
    .bearer_auth(token.trim())
    .build()?;
let body = session.get("https://api.example/me").text().await?;
std::fs::write("me.json", body)?;
# Ok(())
# }
```

## Download a file

`RequestBuilder::download_to(path)` streams directly to disk, commits atomically,
and resumes across dropped transfers when the server sends a strong `ETag`. Pass
`Timeout::new().body(...)` to bound the transfer time (`Duration` alone stops at
the response head for streamed transfers) and `max_body_size` to raise or remove
the 100 MiB cap. See [Streaming](streaming.md#resume-a-download) for checksums,
progress callbacks, and one-shot `Response::download_to`.

```rust,no_run
use std::time::Duration;

use leyline::Timeout;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let bytes = session
    .get("https://example.com/archive.tar.gz")
    .timeout(Timeout::new().body(Duration::from_secs(600)))
    .error_for_status()
    .retry(3)
    .max_body_size(512 * 1024 * 1024)
    .download_to("archive.tar.gz")
    .await?;
println!("{bytes} bytes");
# Ok(())
# }
```

## Find the page for your task

| I want to | Read |
| --- | --- |
| Build a client for a JSON API, end to end | [API clients](api-clients.md) |
| Set a token, base URL, `before_send`, or blocking session | [Sessions](sessions.md) |
| Send headers, forms, `http::Request`, or browser page requests | [Requests](requests.md) |
| Read headers, `Link` pagination, cache hits, or timing | [Responses](responses.md) |
| Stream bodies, resume downloads, or read SSE | [Streaming](streaming.md) |
| Read or stop redirects | [Redirects](redirects.md) |
| Retry failed requests and set timeouts | [Retries and timeouts](retries-and-timeouts.md) |
| Map failures to my own errors or HTTP statuses | [Errors](errors.md) |
| Keep a login between runs | [Cookies](cookies.md) |
| Keep an account's session, tab, and cookies between runs | [Accounts](accounts.md) |
| Crawl many hosts with rate limits and proxy pools | [Crawling](crawling.md) |
| Send traffic through a proxy | [Proxies](proxies.md) |
| Stop a request early | [Cancellation](cancellation.md) |
| Use Leyline inside a web service or Tower stack | [Service integration](service-integration.md) |
| Test against a local server | [Testing](testing.md) |
