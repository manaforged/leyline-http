# API clients

This chapter builds one plain client for a JSON API: a base URL, a token,
timeouts, retries with a rate limit, status errors, pagination, and uploads.
Each section links to the chapter with the full detail.
`examples/api_client.rs` puts the steps together.

## Build the client once

The sections below also cover [response caching](#cache-responses) and
[token refresh](#refresh-a-bearer-token).

```rust,no_run
use std::time::Duration;

use leyline::http::StatusCode;
use leyline::{Retry, Session, Timeout};

const BASE_URL: &str = "https://api.example/v1/";
const USER_AGENT: &str = "my-tool/1.0";
const RATE_REMAINING: &str = "x-ratelimit-remaining";
const RATE_RESET: &str = "x-ratelimit-reset";
const MAX_RATE_WAIT: Duration = Duration::from_secs(120);

# fn run() -> leyline::Result<()> {
let token = std::env::var("API_TOKEN").unwrap_or_default();
let policy = Retry::new(3).wait_header(RATE_RESET).max_wait(MAX_RATE_WAIT).when(|attempt| attempt.is_transient() || attempt.response().is_some_and(|resp| {
        resp.status() == StatusCode::FORBIDDEN && resp.header(RATE_REMAINING) == Some("0")
    }));
let api = Session::builder()
    .base_url(BASE_URL)
    .bearer_auth(&token)
    .headers([("user-agent", USER_AGENT)])
    .headers([("accept", "application/json")])
    .timeout(
        Timeout::new()
            .connect(Duration::from_secs(5))
            .total(Duration::from_secs(180)),
    )
    .retry(policy)
    .build()?;
# drop(api);
# Ok(())
# }
```

| Call | Effect |
| --- | --- |
| `base_url` | Resolves a relative request URL. End it with `/` to keep the last path segment |
| `bearer_auth` | Sends `authorization: Bearer <token>` to the `base_url` origin when a `base_url` is set, and to every host otherwise |
| `headers` | Default headers for every request; a `user-agent` pair replaces `leyline/<version>` |
| `timeout(Timeout)` | `connect` bounds each connection, `total` the whole send |
| `retry` | The default `Retry` of every request |

[Sessions](sessions.md#set-a-token-and-a-base-url) has the URL resolution
rules and the token scope.

`Session` is `Clone`, `Send`, and `Sync`. A clone shares the pool, the cookie
jar, and every setting, so build the client once and clone it into each task.

## Retry transient failures and rate limits

`.retry(3)` retries idempotent methods up to 3 times on a
connection error, a timeout, 429, 502, 503, and 504, with jittered
exponential backoff. The setters in the example extend it:

| Setter | Effect |
| --- | --- |
| `when(rule)` | Replaces the default rule. `attempt.is_transient()` is the default, so `||` adds a case |
| `wait_header(name)`, `wait_header_ms(name)` | Reads the wait from a header, in seconds or in milliseconds. It wins over `Retry-After` |
| `max_wait(d)` | A longer requested wait ends the retries. The default is 60 s |

A request of any method that never reached the server is retried too: a DNS,
connect, TLS, or proxy error, a connect timeout, an HTTP/2 `REFUSED_STREAM`
reset, or an HTTP/3 request the server reports it did not process.

`total` spans every attempt of one send, waits included. A wait that does not
fit in the time left ends the retries, so set `total` above `max_wait`
plus the time of the requests. See
[Retries and timeouts](retries-and-timeouts.md).

## Turn statuses into errors

`error_for_status()` on the request turns a status of 400 or more into a
`Kind::Status` error after the retries. The error keeps the status, the
headers, and up to 64 KiB of the body; see
[Turn a status into an error](responses.md#turn-a-status-into-an-error).
`err.retry_after()` returns the wait the policy read from the last response.
`err.retries_exhausted()` is `true` when the policy wanted another try and
did not make it; see
[Status codes are not errors](errors.md#status-codes-are-not-errors).

```rust,no_run
use leyline::{Error, Kind};

fn explain(err: &Error) -> String {
    match err.kind() {
        Kind::Status => match err.retry_after() {
            Some(wait) => format!("rate limited, retry in {wait:?}"),
            None => format!(
                "api error {:?} request {:?}: {}",
                err.status(),
                err.headers().and_then(|h| h.get("x-request-id")),
                err.body().map(String::from_utf8_lossy).unwrap_or_default()
            ),
        },
        Kind::Decode => format!("unexpected body: {err}"),
        Kind::Timeout => "the api timed out".to_owned(),
        other => format!("{other:?}: {err}"),
    }
}
# let _ = explain;
```

See [Errors](errors.md).

## Read JSON pages

`pages()` sends the request, then follows the `Link: rel="next"` header of
each response with the same settings. See
[Responses](responses.md#follow-link-pagination).

There is no default page limit. Call `limit(max)` to stop after `max` pages.
When a next link goes to another origin, the request drops the session
credentials: the `Authorization`, `Cookie`, and `Proxy-Authorization`
defaults and the session bearer token, as a redirect does.

```rust,no_run
use serde::Deserialize;

#[derive(Deserialize)]
struct Repo {
    name: String,
}

# async fn run(api: leyline::Session) -> leyline::Result<()> {
use futures_util::StreamExt;

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

Response bodies are decoded for you: the default features cover gzip,
deflate, Brotli, and zstd. See [Features and targets](features-and-targets.md).

## Cache responses

Enable a private, in-memory HTTP cache with a byte budget:

```rust,no_run
use leyline::Session;

# async fn run() -> leyline::Result<()> {
let api = Session::builder()
    .base_url("https://api.example/v1/")
    .cache(64 * 1024 * 1024)
    .build()?;
let first = api.get("catalog").bytes().await?;
let again = api.get("catalog").bytes().await?;
api.clear_cache();
# drop((first, again));
# Ok(())
# }
```

Caching is off by default. Session clones share the cache; `to_builder().build()`
creates a new one. The budget charges bodies and a conservative allowance for
headers and metadata, with least-recently-used eviction. An entry larger than
the budget is not stored.

The cache respects cache-control directives, validators, and `Vary`. Stale
responses are revalidated with `If-None-Match` or `If-Modified-Since`; a matching
304 reuses the stored body. Successful writes invalidate all cached variants
of the request URL and same-origin `Location` and `Content-Location` URLs.

Request headers and the proxy route partition entries, so changing credentials
or proxies cannot reuse another context's body. This is deliberately more
conservative than `Vary` alone. Responses that set cookies are not stored.
Streaming requests, ranges, and caller-supplied conditional requests bypass the
cache. A cache miss with `Cache-Control: only-if-cached` returns status 504
without contacting the origin. The cache is not written by `Session::save`. Audited request headers describe
the caller's request before the cache adds revalidation headers; they are not
a packet capture.

## Refresh a bearer token

`refresh_token` takes an asynchronous callback that returns the next access
token. Set both `base_url` and the initial `bearer_auth`; `build()` rejects a
refresh callback without them.

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

A 401 from the base URL origin permits one authentication replay per transport
attempt, only for a replayable body. Concurrent requests that used the old token
share the refresh result, including a failure. An explicit request authorization
header and cross-origin redirects do not trigger refresh. The callback's wait
is covered by the request's total deadline and session shutdown.

Use a separate session for the token endpoint to avoid recursive refresh. The
callback owns provider-specific behavior, refresh-token rotation, and secret
storage. It can be cancelled. Bearer tokens and callbacks are not saved by
`Session::save`.
