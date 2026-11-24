# API clients

This chapter builds one plain client for a JSON API: a base URL, a bearer token,
timeouts, retries and rate limits, status errors, pagination, uploads,
[response caching](#cache-responses), and [token refresh](#refresh-a-bearer-token).
`crates/leyline/examples/api_client.rs` puts the steps together.

## Build the client once

Build one `Session` at startup and clone it across tasks; clones share the
connection pool, cookie jar, cache, and retry budget. `.retry(3)` retries a
`429` (honoring `Retry-After`), a `502`, `503`, or `504`, and transient network
failures. It retries only idempotent methods: `GET`, `HEAD`, `PUT`, `DELETE`,
`OPTIONS`, and `TRACE`. It does not retry `POST` or `PATCH`, to avoid duplicate
writes.

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
| `base_url` | Resolves relative request paths. End it with `/` and write paths without a leading `/`, as in `api.get("repos")` |
| `bearer_auth` | Sends `authorization: Bearer <token>`, scoped to the `base_url` origin |
| `user_agent` | Replaces the default `user-agent` (`leyline/<version>`) |
| `header`, `headers` | Default header(s) sent on every request |
| `timeout(Timeout)` | `connect` bounds opening a connection; `total` bounds the whole call across retries |
| `retry` | Default `Retry` count or policy for every request |

See [Sessions](sessions.md#set-a-token-and-a-base-url) for URL resolution and
token scoping rules.

## Customize retries and rate-limit headers

`.retry(3)` waits longer before each new attempt. It adds random jitter, so
many clients do not retry at the same moment. It honors `Retry-After` up to
`max_wait`, which is 60 s by default.

Some APIs signal rate limits in their own way. For example, GitHub answers
`403` with `x-ratelimit-remaining: 0` and a Unix time in `x-ratelimit-reset`.
Build a `Retry` that reads those headers:

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

With a retry policy set, a request that never reached the server retries
whatever its method. That covers DNS, connect, TLS, and proxy failures, a
connect timeout, and an HTTP/2 or HTTP/3 refused stream. With no retry policy,
nothing retries. Keep `Timeout::total` above `max_wait` plus request time so a
rate-limit wait fits inside the deadline. See
[Retries and timeouts](retries-and-timeouts.md).

## Turn statuses into errors

`4xx` and `5xx` responses return `Ok(Response)` by default. Chain
`.error_for_status()` on a request to turn a status of 400 or more into a
`Kind::Status` error. It is a request setting, not a session setting, and it
applies after retries finish:

- The error keeps the response details listed in [Status codes are not errors](errors.md#status-codes-are-not-errors).
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

Some APIs send the next page in a `Link: <https://api.example/v1/repos?page=2>; rel="next"`
header (RFC 8288). `pages()` follows each `next` link and yields one response
per page. For a cursor or offset in the JSON body, loop and change `.query(..)`
yourself.

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

Leyline decompresses response bodies for each enabled compression feature:
`compression-gzip`, `compression-deflate`, `compression-brotli`, and
`compression-zstd`, all on by default. See
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

Caching is off by default. Only a plain `GET` with no body is cached. The cache
honors `Cache-Control`, `Vary`, and the `ETag` and `Last-Modified` validators.
Session clones share the cache. A session from `to_builder()` gets its own; see
[Derive a session](sessions.md#derive-a-session).

### Cache details

- The budget counts each body plus an allowance for its headers. The least
  recently used entries go first, and an entry larger than the budget is not
  stored.
- A stale entry revalidates with `If-None-Match` or `If-Modified-Since`. A
  `304` reuses the stored body.
- A successful write, such as a `POST`, removes cached entries for its URL.
  It also removes same-origin `Location` and `Content-Location` URLs.
- Request headers and the proxy route split the cache. A request with other
  credentials or another proxy never gets another context's body.
- A response that sets cookies is not stored.
- The cache skips `.stream()` requests and URLs with a user name or password.
  It also skips requests that set `Range`, `If-Range`, `If-None-Match`, or
  `If-Modified-Since`.
- A per-request `.protocol(..)` that differs from the session policy also
  skips the cache.
- A cache miss with `Cache-Control: only-if-cached` returns status 504 and
  sends nothing.
- `Session::save` does not save the cache.
- With `.audit(true)`, `Response::request_headers` shows your request without
  the revalidation headers the cache adds.

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
