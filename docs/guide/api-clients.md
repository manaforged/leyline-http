# API clients

This chapter builds one plain client for a JSON API: a base URL, a bearer token,
timeouts, retries and rate limits, status errors, pagination, uploads,
[response caching](#cache-responses), and [token refresh](#refresh-a-bearer-token).
`examples/api_client.rs` puts the steps together.

## Build the client once

Build one `Session` at startup and clone it across tasks; clones share the
connection pool, cookie jar, cache, and retry budget. `.retry(3)` already
retries `429` (honoring `Retry-After`), `502`, `503`, `504`, and transient
network failures on safe/idempotent methods (`GET`, `HEAD`, `PUT`, `DELETE`,
`OPTIONS`, `TRACE` — not `POST`/`PATCH`, to avoid duplicate writes).

```rust,no_run
use std::time::Duration;

use leyline::{Session, Timeout};

# fn run() -> leyline::Result<()> {
let token = std::env::var("API_TOKEN")
    .map_err(|_| leyline::Error::request("API_TOKEN is not set"))?;
let api = Session::builder()
    .base_url("https://api.example/v1/")
    .bearer_auth(&token)
    .user_agent("my-tool/1.0")
    .header("accept", "application/json")
    .timeout(
        Timeout::new()
            .connect(Duration::from_secs(5))
            .total(Duration::from_secs(180)),
    )
    .retry(3)
    .build()?;
# drop(api);
# Ok(())
# }
```

| Call | Effect |
| --- | --- |
| `base_url` | Resolves relative request paths. End `base_url` with `/` **and** omit a leading `/` on paths (`api.get("repos")`, not `api.get("/repos")`, which strips `/v1/`) |
| `bearer_auth` | Sends `authorization: Bearer <token>` only to the `base_url` origin (or to every host if no `base_url` is set) |
| `user_agent` | Replaces the default `user-agent` (`leyline/<version>`) |
| `header`, `headers` | Default header(s) sent on every request |
| `timeout(Timeout)` | `connect` bounds opening a connection; `total` bounds the whole call across retries |
| `retry` | Default `Retry` count or policy for every request |

See [Sessions](sessions.md#set-a-token-and-a-base-url) for URL resolution and
token scoping rules.

## Customize retries and rate-limit headers

`.retry(3)` waits progressively longer between attempts with random jitter
(so many clients do not retry in lockstep) and honors `Retry-After` up to
`max_wait` (60 s default). Use `Retry::new(n)` when an API uses non-standard
rate-limit headers (such as GitHub's `403` + `x-ratelimit-remaining: 0` + Unix
timestamp in `x-ratelimit-reset`):

```rust,no_run
use std::time::Duration;

use leyline::http::StatusCode;
use leyline::Retry;

const RATE_REMAINING: &str = "x-ratelimit-remaining";
const RATE_RESET: &str = "x-ratelimit-reset";

# fn run() {
let policy = Retry::new(3)
    .wait_header(RATE_RESET)
    .max_wait(Duration::from_secs(120))
    .when(|attempt| {
        attempt.is_transient()
            || attempt.response().is_some_and(|resp| {
                resp.status() == StatusCode::FORBIDDEN
                    && resp.header(RATE_REMAINING) == Some("0")
            })
    });
# drop(policy);
# }
```

| Setter | Effect |
| --- | --- |
| `when(rule)` | Replaces the default rule; combine `attempt.is_transient() \|\| ...` to keep default retries and add a case |
| `any_method(true)` | Also retry `POST` and `PATCH` (use only when your endpoint is idempotent) |
| `wait_header(name)`, `wait_header_ms(name)` | Reads a wait in seconds, Unix timestamp, HTTP date, or milliseconds (takes precedence over `Retry-After`) |
| `max_wait(d)` | If the server asks to wait longer than `d` (default 60 s), stop retrying immediately instead of sleeping |

Requests of any method that never reached the server (DNS, connect, TLS, proxy,
connect timeout, HTTP/2 `REFUSED_STREAM`, or HTTP/3 rejected requests) retry
automatically. Keep `Timeout::total` above `max_wait` plus request time so a
rate-limit wait fits inside the deadline. See
[Retries and timeouts](retries-and-timeouts.md).

## Turn statuses into errors

`4xx` and `5xx` responses return `Ok(Response)` by default. Chain
`.error_for_status()` on each request (it is per-request, not a session
setting) to turn statuses `>= 400` into `Kind::Status` errors after retries
finish:

- Keeps the status, headers, and up to 64 KiB of the body (see [Turn a status into an error](responses.md#turn-a-status-into-an-error)).
- `err.retry_after()` returns the wait from `wait_header` / `Retry-After`, if present.
- `err.retries_exhausted()` is `true` when retries stopped due to max attempts, retry budget, `max_wait`, or remaining `total` time.

```rust,no_run
use leyline::http::StatusCode;
use leyline::{Error, Kind};

fn explain(err: &Error) -> String {
    match (err.kind(), err.status()) {
        (Kind::Status, Some(StatusCode::NOT_FOUND)) => "resource not found".to_owned(),
        (Kind::Status, Some(StatusCode::TOO_MANY_REQUESTS)) => match err.retry_after() {
            Some(wait) => format!("rate limited; retry in {wait:?}"),
            None => "rate limited (no Retry-After header)".to_owned(),
        },
        (Kind::Status, Some(status)) => format!(
            "api error {status} (request {:?}): {}",
            err.headers().and_then(|h| h.get("x-request-id")),
            err.body().map(String::from_utf8_lossy).unwrap_or_default()
        ),
        (Kind::Json | Kind::Decode, _) => format!("unexpected body: {err}"),
        (Kind::Timeout, _) => "the api timed out".to_owned(),
        (other, _) => format!("{other:?}: {err}"),
    }
}
# let _ = explain;
```

See [Errors](errors.md).

## Read JSON pages

For APIs that paginate with RFC 8288 `Link: <https://api.example/v1/repos?page=2>; rel="next"`
headers, `pages()` returns a stream of responses following each `next` link
automatically (for JSON cursor/offset pagination, write a standard `while`
loop updating `.query(...)`).

`pages()` returns a `futures_util::Stream` (no inherent `.next()`), so add
`futures-util = "0.3"` to `Cargo.toml` and import `futures_util::StreamExt`.
There is no default page limit: chain `.take(max)` to cap pages. Cross-origin
next links strip session credentials just like redirects. See
[Responses](responses.md#follow-link-pagination).

```rust,no_run
use futures_util::StreamExt;
use serde::Deserialize;

#[derive(Deserialize)]
struct Repo {
    name: String,
}

# async fn run(api: leyline::Session) -> leyline::Result<()> {
let mut repos = Vec::new();
let mut pages = api.get("repos").error_for_status().pages().take(100);
while let Some(page) = pages.next().await {
    repos.extend(page?.json::<Vec<Repo>>().await?);
}
for repo in &repos {
    println!("{}", repo.name);
}
# Ok(())
# }
```

## Send JSON and upload files

`json(&value)` sends a JSON body. `multipart(form)` sends
`multipart/form-data`; `Part::file(path)?` streams a file from disk, and
`mime` sets the part's `content-type`. See
[Requests](requests.md#upload-multipart-forms).

```rust,no_run
use leyline::multipart::{Form, Part};

# async fn run(api: leyline::Session) -> leyline::Result<()> {
api.post("issues")
    .json(&serde_json::json!({ "title": "hello" }))
    .error_for_status()
    .send().await?;

let form = Form::new()
    .part("title", Part::text("report"))
    .part("file", Part::file("report.pdf")?.mime("application/pdf"));
api.post("uploads").multipart(form).error_for_status().send().await?;
# Ok(())
# }
```

Response bodies are decoded automatically for all enabled compression features
(`gzip`, `deflate`, `br`, `zstd` by default). See
[Cargo features and linking](features-and-targets.md).

## Cache responses

Enable a private, in-memory HTTP cache with a byte budget. Pass `cache(None)` to
turn caching off on a derived session.

```rust,no_run
use leyline::Session;

# async fn run() -> leyline::Result<()> {
let api = Session::builder()
    .base_url("https://api.example/v1/")
    .cache(64 * 1024 * 1024)
    .build()?;
let first = api.get("catalog").send().await?;
let again = api.get("catalog").send().await?;
println!("first={} again={}", first.from_cache(), again.from_cache());
api.clear_cache();
# Ok(())
# }
```

Caching is off by default. Session clones share the cache; `to_builder().build()`
creates a new one. The budget charges bodies plus an allowance for headers and
metadata with LRU eviction, and entries larger than the budget are not stored.

- **Revalidation and invalidation.** Respects `Cache-Control`, validators, and `Vary`. Stale entries revalidate with `If-None-Match` or `If-Modified-Since` (a `304` reuses the stored body). Successful writes invalidate cached variants of the request URL and same-origin `Location` / `Content-Location` URLs.
- **Context partitioning.** Request headers and the proxy route partition entries, so changing credentials or proxies cannot reuse another context's body. Responses that set cookies are not stored.
- **Bypass rules.** The cache is bypassed for non-`GET` methods, `.stream()` requests, requests with a body, URLs with embedded credentials, `Range` / `If-Range` / `If-None-Match` / `If-Modified-Since` request headers, and per-request `.protocol(...)` overrides that differ from the session policy.
- **Directives and persistence.** A cache miss with `Cache-Control: only-if-cached` returns status 504 without contacting the origin. `Session::save` does not save the cache. Audited request headers describe the caller's request before revalidation headers are added.

## Refresh a bearer token

`refresh_token` takes a `Fn() -> Fut + Send + Sync + 'static` callback whose
future returns `leyline::Result<String>` with the next access token. Both
`base_url` and an initial `bearer_auth` are required; `build()` fails with
`Kind::Config` without them.

```rust,no_run
use leyline::Session;
use serde::Deserialize;

#[derive(Deserialize)]
struct Token { access_token: String }

# fn run() -> leyline::Result<()> {
let tokens = Session::new();
let api = Session::builder()
    .base_url("https://api.example/v1/")
    .bearer_auth("initial-access-token")
    .refresh_token(move || {
        let tokens = tokens.clone();
        async move {
            let token = tokens.post("https://auth.example/token")
                .form([("grant_type", "refresh_token"), ("refresh_token", "replace-with-your-refresh-token")])
                .error_for_status().send().await?.json::<Token>().await?;
            Ok(token.access_token)
        }
    })
    .build()?;
# drop(api);
# Ok(())
# }
```

- On a `401` from the `base_url` origin, Leyline refreshes the token and retries the request once (for buffered bodies; streamed bodies cannot be re-sent).
- If multiple concurrent requests hit `401` with the same expired token, only one refresh runs and every waiting caller receives its result (or error).
- Explicit per-request `Authorization` headers and cross-origin redirects do not trigger refresh.
- A caller's wait stops on `Timeout::total`, cancellation, or session shutdown without aborting the shared in-progress refresh task.
- Use a separate `Session` for the token endpoint to avoid recursive refresh. Bearer tokens and refresh callbacks are not written by `Session::save`.
