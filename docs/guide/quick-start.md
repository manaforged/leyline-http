# Quick start

This chapter takes you from an empty project to a request and a parsed
response. Each section links to the chapter that owns the topic.

## Add the dependency

The package is `leyline-http`. The library it builds is `leyline`, so that is
the name you import. Leyline is async and runs on Tokio.

```toml
[dependencies]
leyline-http = "0.1"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

`serde` and `serde_json` are only for the JSON examples. The first build
compiles BoringSSL, so it needs CMake, a C and C++ compiler, and Git. [Supported platforms](platforms.md) lists the targets and the tools.

## Send a GET request

`leyline::get` sends one GET from a plain session. For more than one request,
build a `Session` and reuse it: it holds the connection pool and the cookie
jar.

```rust,no_run
#[tokio::main]
async fn main() -> leyline::Result<()> {
    let resp = leyline::get("https://example.com/").await?;
    println!("{} {}", resp.status(), resp.url());
    Ok(())
}
```

`Session::get` and its siblings return a `RequestBuilder`. Awaiting the
builder sends the request. `.send()` does the same and returns a future you
can hold in a variable.

## Choose a plain or a browser session

| Session | Build it with | Sends |
| --- | --- | --- |
| Plain | `Session::new()` or `Session::builder().build()` | `user-agent: leyline/<version>`, `accept: */*`, and no `sec-*` headers |
| Browser | `Session::browser(Browser::default())` | The TLS, HTTP/2, and headers of the newest bundled Chrome on Windows |
| Pinned browser | `Session::builder().browser(Browser::Chrome154).build()` | One fixed browser profile |

Use a plain session for APIs. Use a browser session when the server checks
that the client is a browser. `SessionBuilder::user_agent` replaces the
default `user-agent` on either kind.

```rust,no_run
use leyline::{Browser, Session};

# fn run() -> leyline::Result<()> {
let plain = Session::builder().user_agent("my-app/1.0").build()?;
let chrome = Session::browser(Browser::default());
let pinned = Session::builder().browser(Browser::Chrome154).build()?;
# drop((plain, chrome, pinned));
# Ok(())
# }
```

[Sessions](sessions.md) lists the headers each kind sends.

## Read the body

`text()` decodes the body with the charset from `Content-Type`, and falls
back to UTF-8. `json::<T>()` deserializes into any `DeserializeOwned` type. A
body that does not parse is an `ErrorCategory::Decode` error. Each body call
consumes the response, so read `status()` and `headers()` first.

```rust,no_run
use serde::Deserialize;

#[derive(Deserialize)]
struct State {
    name: String,
    ready: bool,
}

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let page = session.get("https://example.com/").await?.text().await?;
let state = session
    .get("https://api.example/state")
    .await?
    .json::<State>()
    .await?;
println!("{} {} {}", page.len(), state.name, state.ready);
# Ok(())
# }
```

## Send JSON, a form, and query parameters

`json` serializes the value and sets `content-type: application/json`. `form`
sends `application/x-www-form-urlencoded`. `query` appends pairs to the URL.
`query` and `form` take any iterator of `(key, value)` pairs where both sides
are `AsRef<str>`. `timeout` with a `Duration` bounds the whole request,
retries included; for a streamed body or a download, add
`TimeoutConfig::body`.

```rust,no_run
use std::time::Duration;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let created = session
    .post("https://api.example/items")
    .json(&serde_json::json!({ "name": "leyline" }))
    .timeout(Duration::from_secs(10))
    .await?;
let found = session
    .get("https://api.example/search")
    .query([("q", "leyline"), ("page", "2")])
    .await?;
let login = session
    .post("https://api.example/login")
    .form([("user", "ada"), ("password", "secret")])
    .await?;
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
match session.get("https://api.example/users/7").error_for_status().await {
    Ok(resp) => println!("{}", resp.text().await?),
    Err(err) if err.status() == Some(StatusCode::NOT_FOUND) => {
        println!("no such user: {}", err.body_text().unwrap_or_default());
    }
    Err(err) => return Err(err),
}
# Ok(())
# }
```

`Error::category()` sorts every failure into one `ErrorCategory`, such as
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
let body = session.get("https://api.example/me").await?.text().await?;
std::fs::write("me.json", body)?;
# Ok(())
# }
```

## Download a file

`RequestBuilder::download` streams the decoded body to a file in one atomic
step and returns the number of bytes written. A status of 400 or more is an
error. The limit caps the decoded size; `None` leaves only the session's
`max_body_size` (100 MiB by default). See
[Streaming](streaming.md#download-a-file).

```rust,no_run
use std::time::Duration;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let bytes = session
    .get("https://example.com/archive.tar.gz")
    .timeout(Duration::from_secs(600))
    .download("archive.tar.gz", Some(512 * 1024 * 1024))
    .await?;
println!("{bytes} bytes");
# Ok(())
# }
```

## Find the page for your task

| I want to | Read |
| --- | --- |
| Build a client for a JSON API, end to end | [API clients](api-clients.md) |
| Set a token, a base URL, or share a session across tasks | [Sessions](sessions.md) |
| Send headers, forms, or the requests of a browser page | [Requests](requests.md) |
| Read headers, the final URL, the redirect chain, or timing | [Responses](responses.md) |
| Download a large file with a size limit | [Streaming](streaming.md) |
| Read or stop redirects | [Redirects](redirects.md) |
| Retry failed requests | [Retries and timeouts](retries-and-timeouts.md) |
| Map failures to my own errors or HTTP statuses | [Errors](errors.md) |
| Keep a login between runs | [Cookies](cookies.md) |
| Keep an account's device, tab, and cookies between runs | [Accounts](accounts.md) |
| Crawl many hosts with rate limits and proxy pools | [Crawling](crawling.md) |
| Send traffic through a proxy | [Proxies](proxies.md) |
| Stop a request early | [Cancellation](cancellation.md) |
| Use Leyline inside a web service | [Service integration](service-integration.md) |
| Test against a local server | [Testing](testing.md) |
