# Retries and timeouts

A `Retry` decides whether a whole request runs again, and a `Timeout`
bounds how long each part of it may take. Retries are off by default: a
request is sent once unless you set a retry count. Every timeout except
`total` (300 s), `connect` (10 s), and `error_body` (10 s) is off by default.

## Retry a request

`.retry(n)` retries up to `n` times on the failures that usually pass on
their own: connection errors, timeouts, and status 429, 502, 503, and 504.
The wait between tries doubles from 100 ms to 1 s with equal jitter. A server
can ask for a wait of up to 60 s.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::builder().retry(3).build()?;
let resp = session.get("https://example.com/flaky").retry(4).send().await?;
# let _ = resp;
# Ok(())
# }
```

`SessionBuilder::retry` sets the session default. `RequestBuilder::retry`
replaces it for one request. `.retry(0)` turns retries off. Both take a
number or a `Retry`.

## Choose what to retry

`Retry::new(n).when(rule)` replaces the default rule. The rule gets an
`Attempt` and returns `true` to try again:

| `Attempt` | Returns |
| --- | --- |
| `status()` | The response status, or `None` after an error |
| `response()`, `error()` | The response or the error of this try |
| `is_transient()` | The default rule: `Error::is_retryable()`, or status 429, 502, 503, or 504 |
| `is_block()` | `response().block()` is `Some`; see [blocks](crawling.md#blocks-retries-and-proxies) |

```rust
use leyline::Retry;

let also_408 = Retry::new(3).when(|attempt| {
    attempt.is_transient() || attempt.status().is_some_and(|s| s.as_u16() == 408)
});
let not_blocks = Retry::new(3).when(|attempt| attempt.is_transient() && !attempt.is_block());
# let _ = (also_408, not_blocks);
```

See [Predicates](errors.md#predicates) for the exact `Error::is_retryable()`
rules. `Kind::Overloaded` and local `Kind::Body` stream failures are never
retried by default.

## Backoff and retry budget

`Retry::backoff(min, max)` waits between half and all of `min * 2^n` before
attempt `n + 1`, capped at `max` (equal jitter). Call `retry.delay(attempt)` to
sample the wait for a 0-based retry index.

`Retry::budget(ratio)` limits retries during an outage with a token bucket.
The bucket starts full, with 100 tokens. Each new request adds `ratio` tokens,
for example `0.2` for 20%, and the bucket never holds more than 100. Each
retry spends 1 token.

Set the budget on `SessionBuilder::retry` so that all clones share one bucket.
A per-request `Retry::budget` creates its own bucket. When the budget stops a
retry of a status that `RequestBuilder::error_for_status()` then rejects,
`Error::retries_exhausted()` is `true`.

```rust
use std::time::Duration;

use leyline::Retry;

let retry = Retry::new(5)
    .backoff(Duration::from_millis(50), Duration::from_secs(2))
    .budget(0.2);
# let _ = retry;
```

## Retry-After wins

When a retried response carries `Retry-After`, its value replaces the
backoff. Leyline reads delta-seconds and an IMF-fixdate with a `GMT` or `UTC`
zone. An unparsable value falls back to the backoff. A header set with
[`wait_header`](#wait-on-a-header) wins over `Retry-After`.

`max_wait` caps the wait; the default is 60 s. The server can ask for longer
than the cap, or a wait can be longer than the time left before the `total`
timeout. Then Leyline stops and returns the response, as when retries run
out. With `error_for_status()`, the status error has `retries_exhausted()`
set to `true`. A transport error never sets it.

A `Retry-After` on a response the rule does not retry is not acted on. Read
it after `error_for_status()` with `Error::retry_after()`:

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
match session.get("https://api.example/jobs").error_for_status().send().await {
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

## Wait on a header

`wait_header(name)` names a header that says how long to wait. Leyline reads
any of these forms:

| Value | Example |
| --- | --- |
| Seconds to wait | `30` |
| The Unix time to wait until: any number of at least `1000000000`. A past time means no wait | `1767225600` |
| The time to wait until, as an IMF-fixdate | `Thu, 01 Jan 2026 00:00:00 GMT` |

`wait_header_ms(name)` names a header that holds milliseconds to wait.

Leyline reads the `wait_header` headers in the order you added them, then
`Retry-After`, and uses the first value that parses. `max_wait` caps these
waits too. A retry after an error has no response, so it uses the backoff.

This retries a 403 that reports a used-up quota, and only that one, after
the reset time:

```rust,no_run
use std::time::Duration;

use leyline::Retry;
use leyline::http::StatusCode;

const REMAINING: &str = "x-ratelimit-remaining";
const RESET: &str = "x-ratelimit-reset";

# async fn run() -> leyline::Result<()> {
let retry = Retry::new(3)
    .when(|attempt| {
        attempt.response().is_some_and(|resp| {
            resp.status() == StatusCode::FORBIDDEN && resp.header(REMAINING) == Some("0")
        })
    })
    .wait_header(RESET)
    .max_wait(Duration::from_secs(120));
let session = leyline::Session::builder().retry(retry).build()?;
let resp = session.get("https://api.example/repos").error_for_status().send().await?;
# let _ = resp;
# Ok(())
# }
```

Blocks and retries through other proxies are covered in
[Crawling](crawling.md#blocks-retries-and-proxies).

## The idempotency rule

Leyline retries only idempotent methods by default: GET, HEAD, OPTIONS, PUT,
DELETE, and TRACE (RFC 9110, section 9.2.2). A retry also needs a body that
can be sent again. A streaming request body is retried only when no byte of
it was read, for example after a connect error. See
[Streaming](streaming.md).

Some failures mean the request never reached the server. Leyline retries
these for any method:

- DNS, connect, TLS, and proxy errors, and connect timeouts.
- HTTP/2 `REFUSED_STREAM`, and HTTP/3 requests the server reports it did not process.
- A request that answers a Digest `401` challenge and fails before it reaches the server.

An error on a redirect hop does not count as unsent, because the first
request already got a response.

`any_method(true)` retries every failure of any method. Use it only when the
endpoint is safe to repeat.

## Count the attempts

`Response::attempts()` returns the number of attempts that produced the
response: 1 without retries. When retries run out on a retryable status, the
response carries the full count. `Error::attempts()` and
the `attempts` field of `leyline::trace::Event::Summary` carry the same count. See
[Logging and tracing](logging.md#one-event-per-request).

## Retries and streamed responses

A `.stream()` request uses the same policy, and the decision is made on the
response head. After `send()` returns, a failure while you read the body is
not retried. To try again, send a new request.

## Resends outside the policy

Connection setup has no retry loop of its own. Leyline makes these resends
itself, before `Retry` sees the outcome:

- A stale pooled connection. On HTTP/1.1 or HTTP/2, a reused connection can fail before a response. An idempotent request with a buffered or empty body then runs once more on a new connection. This happens only on the first attempt.
- An unsent or rejected stream. A buffered or empty request of any method resends once per attempt after HTTP/2 `REFUSED_STREAM`. On HTTP/3 it resends after `GOAWAY` (on a new connection) or after `H3_REQUEST_REJECTED` before headers (on the same connection).
- A `Protocol::Race` fallback. If HTTP/3 fails and the TCP/TLS connection negotiated HTTP/1.1, Leyline uses HTTP/1.1. If both fail, Leyline returns the TCP/TLS error and does not dial again.

The HTTP version is chosen during the TLS handshake (ALPN), so it never
causes a resend. See
[One handshake per connection](network.md#one-handshake-per-connection).

## The timeouts

`Timeout` holds all six timeouts. Each setter takes a `Duration`, or `None`
to turn that timeout off.

| Setter | Default | Covers |
| --- | --- | --- |
| `total` | 300 s | Whole `send`: redirects, retries, backoff, and buffered body read (ends at response headers for `.stream()`). Also bounds WebSocket handshakes and `RequestBuilder::download_to` retries |
| `connect` | 10 s | One new connection (DNS, TCP/UDP, proxy negotiation, TLS/QUIC handshake). Session-wide; not applied to pooled reuse |
| `response_header` | none | Dispatch to response headers per redirect leg, including connect (on buffered requests, also covers reading the buffered body) |
| `read` | none | Longest idle gap between chunks on streamed reads (`.stream()`, `download_to`, `events()`, `bytes_stream()`). Not applied to async buffered responses (on `build_blocking()`, `read` or `total` also sets the HTTP/1.1 socket I/O timeout) |
| `body` | none | Total time to read a streamed body (`.stream()`, `download_to`, `events()`, `bytes_stream()`) from its first read. Not applied to buffered responses |
| `error_body` | 10 s | Body prefix captured by `RequestBuilder::error_for_status()`, bounded by `total`. See [Errors](errors.md#status-codes-are-not-errors) |

To keep timer cost low, a `total` timeout of 2 s or more is checked on a
shared 250 ms ticker. It can fire up to about 250 ms late. A shorter `total`
uses an exact timer.

Suggested settings by workload:

- For buffered JSON APIs and service calls, set `connect` (5 to 10 s), `response_header` (a cap per attempt), and `total` (a cap across retries).
- For streamed relays and downloads, clear or raise `total`. Set `read` to catch a stalled server and `body` to catch a slow trickle.

`total`, and `.timeout(duration)`, bound the whole call, with every retry and
wait. To bound each attempt instead, set `response_header` or `read`.

```rust,no_run
use leyline::{Session, Timeout};
use std::time::Duration;

# fn run() -> leyline::Result<()> {
let session = Session::builder()
    .timeout(
        Timeout::new()
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

`SessionBuilder::timeout` takes a `Duration` or a `Timeout`. A
`Duration` sets `total`, and the other settings keep their
defaults. A later `timeout` call replaces the whole configuration, so to
change `connect` alone, pass `Timeout::new().connect(duration)`.

Every timeout gives an error of kind `Kind::Timeout`. For a connect timeout,
`is_connect()` is also true; see [Errors](errors.md#match-on-the-kind).

## Per-request timeouts

`RequestBuilder::timeout` and a `Timeout` extension on an `http::Request` do
not replace the session's `Timeout`. They merge the fields you set over the
session values. A field set to `None` turns that timeout off. An unset field
keeps the session value.

A plain `Duration` sets `total`. A per-request `connect` timeout is ignored,
because connections are pooled across the session.

```rust,no_run
use leyline::Timeout;
use std::time::Duration;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();

let quick = session
    .get("https://example.com/slow")
    .timeout(Duration::from_secs(5))
    .send().await?;

let streamed = session
    .get("https://example.com/feed")
    .stream()
    .timeout(
        Timeout::new()
            .total(None)
            .read(Duration::from_secs(10))
            .response_header(Duration::from_secs(15)),
    )
    .send().await?;

# let _ = (quick, streamed);
# Ok(())
# }
```

## Bound a whole download

`Timeout::body` bounds a download from its first read, for both
`RequestBuilder::download_to` and `Response::download_to`. A resumable
download is still bounded by `total` across its retries. Clear or raise
`total` for a long download. [Streaming](streaming.md#download-a-file) has
the example, and [Cancellation](cancellation.md) covers partial files.
