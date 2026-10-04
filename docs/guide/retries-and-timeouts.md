# Retries and timeouts

A `RetryPolicy` decides whether a whole request runs again, and a
`TimeoutConfig` bounds how long each part of it may take. Retries are off by
default: `RetryPolicy::default()` is `RetryPolicy::none()`, so a request is
sent once unless you ask for more. Every timeout except `total` and `connect`
is off by default.

## The transient policy

`RetryPolicy::transient()` covers the failures that usually pass on their own:
connection errors, timeouts, and status 429, 502, 503, and 504. It allows 3
retries, so 4 attempts in total, with exponential backoff from 100 ms to 1 s,
factor 2, and full jitter. A server can ask for a wait of up to 60 s;
`max_retry_after` changes that cap.

```rust,no_run
use leyline::RetryPolicy;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .retry(RetryPolicy::transient())
    .build()?;
let resp = session
    .get("https://example.com/flaky")
    .retry(RetryPolicy::transient().max_retries(4))
    .await?;
# let _ = resp;
# Ok(())
# }
```

`SessionBuilder::retry` sets the session default. `RequestBuilder::retry`
replaces it for one request.

## Custom triggers

`RetryTrigger` has four variants:

| Trigger | Matches |
| --- | --- |
| `Status(u16)` | A response with that status |
| `ServerError` | A response with any 5xx status |
| `Timeout` | An error for which `Error::is_timeout()` is true |
| `ConnectionError` | Any other error for which `Error::is_retryable()` is true: a failed connect, a reset, aborted, or closed connection, an I/O error on the way to a proxy, and a proxy answer of 502, 503, or 504 to `CONNECT` |

A `CONNECT` answer ends the request with an error, not a response, so only
`ConnectionError` matches it. Other `CONNECT` answers, such as 407, and a
SOCKS5 reply from 3 to 6, are not retried. No other error is retried.

Start from `none()` or `transient()`. `retry_on(triggers)` replaces the
trigger set and `on_status(code)` adds one status. Every other setting has a
setter named after it: `max_retries`, `initial_backoff`, `max_backoff`,
`backoff_factor`, `jitter`, `max_retry_after`, and `allow_non_idempotent`.

```rust
use leyline::{RetryPolicy, RetryTrigger};
use std::time::Duration;

let policy = RetryPolicy::none()
    .max_retries(5)
    .initial_backoff(Duration::from_millis(50))
    .max_backoff(Duration::from_secs(2))
    .backoff_factor(1.5)
    .jitter(true)
    .retry_on([RetryTrigger::ServerError, RetryTrigger::ConnectionError]);

let also = RetryPolicy::transient().on_status(408);
# let _ = (policy, also);
```

Backoff for attempt `n` is `initial_backoff * backoff_factor.powi(n)`, capped at
`max_backoff`. With `jitter` set, the result is multiplied by a uniform random
factor between 0 and 1 (full jitter). `RetryPolicy::backoff(attempt)` returns
that value, so a retry loop of your own can wait the same way. With `jitter`
on, each call draws a new factor.

```rust
use leyline::RetryPolicy;
use std::time::Duration;

let policy = RetryPolicy::transient().jitter(false);
assert!(policy.backoff(1) > policy.backoff(0));
assert_eq!(policy.backoff(10), Duration::from_secs(1));
```

## Retry-After wins

When a retried response carries `Retry-After`, its value replaces the
computed backoff. Leyline reads delta-seconds and an IMF-fixdate with a `GMT`
or `UTC` zone. An unparsable value falls back to the computed backoff. A
header set with [`wait_header`](#retry-on-a-condition-and-wait-on-a-header)
wins over `Retry-After`.

`max_retry_after` caps the wait. `RetryPolicy::transient()` sets it to 60 s,
and `RetryPolicy::none()` has no cap. When the server asks for longer than
the cap, or a wait or backoff is longer than the time left before the `total`
timeout, Leyline stops and returns the response, or the last error, as when
retries run out. `Error::retries_exhausted()` is then `true`.

A `Retry-After` on a status that no trigger or `retry_if` matches is not
acted on. Read it after `error_for_status()` with `Error::retry_after()`:

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
match session.get("https://api.example/jobs").error_for_status().await {
    Ok(resp) => println!("{}", resp.status()),
    Err(err) => match err.retry_after() {
        Some(wait) => println!("come back in {wait:?}"),
        None => return Err(err),
    },
}
# Ok(())
# }
```

See [Status codes are not errors](errors.md#status-codes-are-not-errors).

## Retry on a condition and wait on a header

`retry_if(predicate)` adds a test on the response head: the status and the
headers. A response is retried when a trigger matches its status or when any
predicate returns `true`.

`wait_header(name, format)` names a header that says how long to wait.
`WaitFormat` says how to read it:

| `WaitFormat` | Value | Example |
| --- | --- | --- |
| `Seconds` | Seconds to wait, whole or fractional | `30` |
| `UnixSeconds` | The Unix time to wait until. A past time means no wait | `1767225600` |
| `HttpDate` | The time to wait until, as an IMF-fixdate | `Thu, 01 Jan 2026 00:00:00 GMT` |

Leyline reads the `wait_header` headers in the order you added them, then
`Retry-After`, and uses the first value that parses. `max_retry_after` caps
these waits too. They apply to every retried response, whichever trigger or
predicate matched it. A retry after an error has no response, so it uses the
computed backoff.

This policy retries a 403 that reports a used-up quota, and only that one,
after the reset time:

```rust,no_run
use std::time::Duration;

use leyline::{RetryPolicy, WaitFormat};
use leyline::http::StatusCode;

const REMAINING: &str = "x-ratelimit-remaining";
const RESET: &str = "x-ratelimit-reset";

# async fn run() -> leyline::Result<()> {
let policy = RetryPolicy::transient()
    .retry_if(|resp| {
        resp.status() == StatusCode::FORBIDDEN && resp.header(REMAINING) == Some("0")
    })
    .wait_header(RESET, WaitFormat::UnixSeconds)
    .max_retry_after(Duration::from_secs(120));
let session = leyline::Session::builder().retry(policy).build()?;
let resp = session.get("https://api.example/repos").error_for_status().await?;
# let _ = resp;
# Ok(())
# }
```

A policy with `max_retries(0)`, such as `none()`, never retries, so start
from `transient()` or set `max_retries`.

Blocks, `skip_blocks`, and retries through other proxies are covered in
[Crawling](crawling.md#retry-a-block-through-another-proxy).

## The idempotency rule

Leyline retries only idempotent methods by default: GET, HEAD, OPTIONS, PUT,
DELETE, and TRACE (RFC 9110, section 9.2.2). A POST or PATCH is sent once. A
retry also needs a body that can be sent again. A streaming request body is
retried only when no byte of it was read, for example after a connect error.
See [Streaming](streaming.md).

`retry_unsent(true)` retries any method when the error happened before the
request was sent: a DNS, connect, TLS, or proxy error, a connect timeout, an
HTTP/2 `REFUSED_STREAM` reset, or an HTTP/3 request that the server reports
it did not process. An HTTP/3 request that the server rejects twice is
retried on the same connection. The server never processed the request, so a retry cannot
repeat a write. The error
must still match a trigger. After a redirect or an authentication challenge
got a response, an error on a later leg does not count: the server already
answered the original request, so `retry_unsent` does not replay it.

```rust,no_run
use leyline::RetryPolicy;

# fn run() -> leyline::Result<()> {
let policy = RetryPolicy::transient().retry_unsent(true);
let session = leyline::Session::builder().retry(policy).build()?;
# drop(session);
# Ok(())
# }
```

`allow_non_idempotent(true)` retries every failure of any method. Use it only
when the endpoint is safe to repeat.

## Count the attempts

`Response::attempts()` returns the number of attempts that produced the
response: 1 without retries. When retries run out on a retryable status, the
response carries the full count. `Error::attempts()` and
`trace::Summary::attempts` carry the same count. See
[Logging and tracing](logging.md#one-event-per-request).

## Retries and streamed responses

A `.stream()` request uses the same policy, and the decision is made on the
response head. After `send()` returns, a failure while you read the body is
not retried. To try again, send a new request.

## Resends outside the policy

Connection setup has no retry loop of its own. These cases send a request or
connect again without asking the policy. Each happens at most once for one
attempt, and the result then goes to `RetryPolicy` like any other:

- HTTP/1.1: a pooled keep-alive connection fails before the response. An
  idempotent request with a buffered or empty body goes once more on a new
  connection.
- HTTP/2: a pooled connection fails before the response. A request with a
  buffered or empty body goes once more on a new connection when the method
  is idempotent, or for any method after a `REFUSED_STREAM` reset.
- HTTP/3: a pooled connection reports that it did not send the request, for
  example after `GOAWAY`. A request with a buffered or empty body goes once
  more on a new connection, for any method. After an `H3_REQUEST_REJECTED`
  reset before the response head, a buffered request goes once more on the
  same connection.
- `ProtocolPolicy::Race`: when both the HTTP/3 and the HTTP/2 connect fail,
  Leyline falls back to `Auto` and connects again.

ALPN needs no second send. See
[One handshake per connection](network.md#one-handshake-per-connection).

## The timeouts

`TimeoutConfig` holds every timeout. Each setter takes a `Duration` or
`None`, and `None` turns that timeout off.

| Setter | Default | Covers |
| --- | --- | --- |
| `total` | 300 s | One `send`: every redirect, retry, backoff sleep, and buffered body read. For a `.stream()` response it ends at the response head. Expiry gives `Kind::Timeout` |
| `connect` | 10 s | One new connection: DNS, TCP connect, proxy negotiation, and TLS, or on HTTP/3 the DNS lookup, UDP setup, and QUIC handshake. Each new connection gets a full window, and `total` bounds the sum. Pooled reuse is not covered |
| `response_header` | none | From dispatch to the transport response, per redirect, connection setup included. On a buffered response the body arrives inside this window |
| `read` | none | The longest gap between two body chunks of a `.stream()` response, or of a buffered body that arrives in pieces |
| `body` | none | The whole body, streamed or buffered, counted from the first read of the body |
| `error_body` | 10 s | The body that `RequestBuilder::error_for_status()` keeps in a status error. `total` also bounds it. See [Responses](responses.md#turn-a-status-into-an-error) |

```rust,no_run
use leyline::{Session, TimeoutConfig};
use std::time::Duration;

# fn run() -> leyline::Result<()> {
let session = Session::builder()
    .timeout(
        TimeoutConfig::new()
            .total(Duration::from_secs(60))
            .connect(Duration::from_secs(5))
            .read(Duration::from_secs(15))
            .response_header(Duration::from_secs(20))
            .body(Duration::from_secs(120)),
    )
    .build()?;
# let _ = session;
# Ok(())
# }
```

`SessionBuilder::timeout` takes a `Duration` or a `TimeoutConfig`. A
`Duration` sets `total`, and the other settings keep their
defaults. A later `timeout` call replaces the whole configuration, so to
change `connect` alone, pass `TimeoutConfig::new().connect(duration)`.

Every timeout gives an error whose `is_timeout()` is true. The kind depends
on which one elapsed; see [Fine-grained kinds](errors.md#fine-grained-kinds).

## Per-request timeouts

`RequestBuilder::timeout` merges over the session timeouts, setting by
setting. A setting that the request sets wins, `None` included, and the rest
keep the session value. A `Duration` sets `total`. `connect` stays
session-wide, because connections are pooled across requests.

```rust,no_run
use leyline::TimeoutConfig;
use std::time::Duration;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();

let quick = session
    .get("https://example.com/slow")
    .timeout(Duration::from_secs(5))
    .await?;

let streamed = session
    .get("https://example.com/feed")
    .stream()
    .timeout(
        TimeoutConfig::new()
            .total(None)
            .read(Duration::from_secs(10))
            .response_header(Duration::from_secs(15)),
    )
    .await?;

# let _ = (quick, streamed);
# Ok(())
# }
```

## Bound a whole download

`TimeoutConfig::body` bounds the whole body from its first read, so it bounds a
`.stream()` body and `RequestBuilder::download`, which streams. A plain
`Duration` passed to `.timeout` sets only `total`, which ends at the head of
a streamed response, so set `body` for a download. No outer timeout is
needed.

```rust,no_run
use std::time::Duration;

use leyline::TimeoutConfig;

# async fn run(session: leyline::Session) -> leyline::Result<()> {
let written = session
    .get("https://example.com/archive.zip")
    .timeout(TimeoutConfig::new().body(Duration::from_secs(600)))
    .download("archive.zip", None)
    .await;
match written {
    Ok(bytes) => println!("{bytes} bytes"),
    Err(err) if err.is_timeout() => println!("download took too long"),
    Err(err) => return Err(err),
}
# Ok(())
# }
```

A failed or cancelled download removes its temporary `.part` file and leaves
the target path as it was. See [Streaming](streaming.md) and
[Cancellation](cancellation.md).

## Next

Read [Errors](errors.md) to match on `ErrorCategory` and `Kind`.
