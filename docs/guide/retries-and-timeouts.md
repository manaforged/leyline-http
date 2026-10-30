# Retries and timeouts

A `Retry` decides whether a whole request runs again, and a `Timeout`
bounds how long each part of it may take. Retries are off by default: a
request is sent once unless you set a retry count. Every timeout except
`total` and `connect` is off by default.

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

`Retry::new(n).when(rule)` replaces the default rule. The rule gets a
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

`Error::is_retryable()` is true for a failed connect, a timeout, a reset,
aborted, or closed connection, an I/O error on the way to a proxy, and a
proxy answer of 502, 503, or 504 to `CONNECT`.

## Backoff

`Retry::backoff(min, max)` waits between half of and all of
`min * 2^n` before try `n + 1`, capped at `max` (equal jitter). The fixed
half bounds how fast retries come; the random half spreads them out.

```rust
use std::time::Duration;

use leyline::Retry;

let retry = Retry::new(5).backoff(Duration::from_millis(50), Duration::from_secs(2));
# let _ = retry;
```

## Retry-After wins

When a retried response carries `Retry-After`, its value replaces the
backoff. Leyline reads delta-seconds and an IMF-fixdate with a `GMT` or `UTC`
zone. An unparsable value falls back to the backoff. A header set with
[`wait_header`](#wait-on-a-header) wins over `Retry-After`.

`max_wait` caps the wait; the default is 60 s. When the server asks for
longer than the cap, or a wait is longer than the time left before the
`total` timeout, Leyline stops and returns the response, or the last error,
as when retries run out. `Error::retries_exhausted()` is then `true`.

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

A request of any method that never reached the server is retried when the
rule allows it: a DNS, connect, TLS, or proxy error, a connect timeout, an
HTTP/2 `REFUSED_STREAM` reset, or an HTTP/3 request that the server reports
it did not process. The server never processed it, so a retry cannot repeat
a write. After a redirect got a response, an error on a later leg does not
count: the server may already have processed the original request. A 401
digest challenge is not an answer, so an error on the authorized leg still
counts.

`any_method(true)` retries every failure of any method. Use it only when the
endpoint is safe to repeat.

## Count the attempts

`Response::attempts()` returns the number of attempts that produced the
response: 1 without retries. When retries run out on a retryable status, the
response carries the full count. `Error::attempts()` and
the `attempts` field of `Event::Summary` carry the same count. See
[Logging and tracing](logging.md#one-event-per-request).

## Retries and streamed responses

A `.stream()` request uses the same policy, and the decision is made on the
response head. After `send()` returns, a failure while you read the body is
not retried. To try again, send a new request.

## Resends outside the policy

Connection setup has no retry loop of its own. These cases send a request or
connect again without asking the policy. Each happens at most once for one
attempt, and the result then goes to `Retry` like any other:

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
- `Protocol::Race`: when both the HTTP/3 and the HTTP/2 connect fail,
  Leyline falls back to `Auto` and connects again.

ALPN needs no second send. See
[One handshake per connection](network.md#one-handshake-per-connection).

## The timeouts

`Timeout` holds every timeout. Each setter takes a `Duration` or
`None`, and `None` turns that timeout off.

| Setter | Default | Covers |
| --- | --- | --- |
| `total` | 300 s | One `send`: every redirect, retry, backoff sleep, and buffered body read. For a `.stream()` response it ends at the response head. Expiry gives `Kind::Timeout` |
| `connect` | 10 s | One new connection: DNS, TCP connect, proxy negotiation, and TLS, or on HTTP/3 the DNS lookup, UDP setup, and QUIC handshake. Each new connection gets a full window, and `total` bounds the sum. Pooled reuse is not covered |
| `response_header` | none | From dispatch to the transport response, per redirect, connection setup included. On a buffered response the body arrives inside this window |
| `read` | none | The longest gap between two body chunks of a `.stream()` response, or of a body that the transport delivers in pieces. It does not apply to a body that the transport buffers; `response_header` and `total` bound that body |
| `body` | none | The whole body of a `.stream()` response, or of a body that the transport delivers in pieces, counted from the first read of the body. It does not apply to a body that the transport buffers; `response_header` and `total` bound that body |
| `error_body` | 10 s | The body that `RequestBuilder::error_for_status()` keeps in a status error. `total` also bounds it. See [Responses](responses.md#turn-a-status-into-an-error) |

`Timeout::total`, and `.timeout(duration)`, bound the whole call, every retry and wait included. To bound each attempt, set `Timeout::response_header` or `read`.

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

`RequestBuilder::timeout` merges over the session timeouts, setting by
setting. A setting that the request sets wins, `None` included, and the rest
keep the session value. A `Duration` sets `total`. `connect` stays
session-wide, because connections are pooled across requests.

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

`Timeout::body` bounds the whole body from its first read, so it bounds a
`.stream()` body and `Response::download_to`, which streams. A plain
`Duration` passed to `.timeout` sets only `total`, which ends at the head of
a streamed response, so set `body` for a download. No outer timeout is
needed.

```rust,no_run
use std::time::Duration;

use leyline::Timeout;

# async fn run(session: leyline::Session) -> leyline::Result<()> {
let written = session
    .get("https://example.com/archive.zip")
    .timeout(Timeout::new().body(Duration::from_secs(600)))
    .error_for_status()
    .stream()
    .send()
    .await?
    .download_to("archive.zip")
    .await;
match written {
    Ok(bytes) => println!("{bytes} bytes"),
    Err(err) if err.kind() == leyline::Kind::Timeout => println!("download took too long"),
    Err(err) => return Err(err),
}
# Ok(())
# }
```

A failed or cancelled download removes its temporary `.part` file and leaves
the target path as it was. See [Streaming](streaming.md) and
[Cancellation](cancellation.md).

## Next

Read [Errors](errors.md) to match on `Kind`.
