# API clients

This chapter builds one plain client for a JSON API: a base URL, a token,
timeouts, retries with a rate limit, status errors, pagination, and uploads.
Each section links to the chapter with the full detail.
`examples/api_client.rs` puts the steps together.

## Build the client once

```rust,no_run
use std::time::Duration;

use leyline::http::StatusCode;
use leyline::{RetryPolicy, Session, TimeoutConfig, WaitFormat};

const BASE_URL: &str = "https://api.example/v1/";
const USER_AGENT: &str = "my-tool/1.0";
const RATE_REMAINING: &str = "x-ratelimit-remaining";
const RATE_RESET: &str = "x-ratelimit-reset";
const MAX_RATE_WAIT: Duration = Duration::from_secs(120);

# fn run() -> leyline::Result<()> {
let token = std::env::var("API_TOKEN").unwrap_or_default();
let policy = RetryPolicy::transient()
    .retry_if(|resp| {
        resp.status() == StatusCode::FORBIDDEN && resp.header(RATE_REMAINING) == Some("0")
    })
    .wait_header(RATE_RESET, WaitFormat::UnixSeconds)
    .max_retry_after(MAX_RATE_WAIT)
    .retry_unsent(true);
let api = Session::builder()
    .base_url(BASE_URL)
    .bearer_auth(&token)
    .user_agent(USER_AGENT)
    .headers([("accept", "application/json")])
    .timeout(
        TimeoutConfig::new()
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
| `user_agent` | Replaces `leyline/<version>` |
| `headers` | Default headers for every request |
| `timeout(TimeoutConfig)` | `connect` bounds each connection, `total` the whole send |
| `retry` | The default `RetryPolicy` of every request |

[Sessions](sessions.md#set-a-token-and-a-base-url) has the URL resolution
rules and the token scope.

`Session` is `Clone`, `Send`, and `Sync`. A clone shares the pool, the cookie
jar, and every setting, so build the client once and clone it into each task.

## Retry transient failures and rate limits

`RetryPolicy::transient()` retries idempotent methods up to 3 times on a
connection error, a timeout, 429, 502, 503, and 504, with jittered
exponential backoff. The setters in the example extend it:

| Setter | Effect |
| --- | --- |
| `retry_if(predicate)` | Also retries a response for which the predicate returns `true` |
| `wait_header(name, format)` | Reads the wait from a header. It wins over `Retry-After` |
| `max_retry_after(d)` | A longer requested wait ends the retries. The default is 60 s |
| `retry_unsent(true)` | Retries any method when the server did not process the request: a DNS, connect, TLS, or proxy error, a connect timeout, an HTTP/2 `REFUSED_STREAM` reset, or an HTTP/3 request the server reports it did not process |

`total` spans every attempt of one send, waits included. A wait that does not
fit in the time left ends the retries, so set `total` above `max_retry_after`
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
use leyline::{Error, ErrorCategory};

fn explain(err: &Error) -> String {
    match err.category() {
        ErrorCategory::Status => match err.retry_after() {
            Some(wait) => format!("rate limited, retry in {wait:?}"),
            None => format!(
                "api error {:?} request {:?}: {}",
                err.status(),
                err.header("x-request-id"),
                err.body_text().unwrap_or_default()
            ),
        },
        ErrorCategory::Decode => format!("unexpected body: {err}"),
        ErrorCategory::Timeout => "the api timed out".to_owned(),
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

```rust,no_run
use serde::Deserialize;

#[derive(Deserialize)]
struct Repo {
    name: String,
}

# async fn run(api: leyline::Session) -> leyline::Result<()> {
let mut repos = Vec::new();
let mut pages = api.get("repos").error_for_status().pages();
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
    .await?;

let form = Form::new()
    .text("title", "report")
    .part("file", Part::file("report.pdf")?.mime("application/pdf"));
api.post("uploads").multipart(form).error_for_status().await?;
# Ok(())
# }
```

Response bodies are decoded for you: the default features cover gzip,
deflate, Brotli, and zstd. See [Features and targets](features-and-targets.md).
