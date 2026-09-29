# Retries and timeouts

## Retries are off by default

`RetryPolicy::default()` is `RetryPolicy::none()`: zero retries, no triggers.
A request is sent once unless you ask for more.

## The transient policy

`RetryPolicy::transient()` covers the failures that usually pass on their own.
It retries connection errors, status 429, 502, 503, and 504, and timeouts. It
allows 3 retries, so 4 attempts in total, with exponential backoff from 100 ms
to 1 s, factor 2, and full jitter.

```rust,no_run
use leyline::RetryPolicy;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session
    .get("https://example.com/flaky")
    .retry(RetryPolicy::transient().max_retries(4))
    .await?;
# let _ = resp;
# Ok(())
# }
```

Set a session-wide default with `SessionBuilder::retry`. Every request inherits
it unless the request sets its own with `RequestBuilder::retry`.

## Custom triggers

`RetryTrigger` has four variants: `ConnectionError`, `Status(u16)`,
`ServerError` for any 5xx, and `Timeout`. Start from `none()` or `transient()`.
`retry_on(triggers)` replaces the trigger set. Every other setting has a
setter named after it: `max_retries`, `initial_backoff`, `max_backoff`,
`backoff_factor`, `jitter`, `max_retry_after`, and `allow_non_idempotent`.

`Status(u16)` and `ServerError` match the status of a response. `Timeout`
matches an error for which `Error::is_timeout()` is true. `ConnectionError`
matches any other error for which `Error::is_retryable()` is true: a failed
connect, a reset, aborted, or closed connection, an I/O error on the way to a
proxy, and a proxy that answers `CONNECT` with 502, 503, or 504. The `CONNECT`
answer ends the request with an error, so only `ConnectionError` matches it.

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

The setters adjust an existing policy. `on_status` adds a status code to the
trigger set.

`RetryPolicy` decides whether a whole request runs again. Connection setup has
no retry loop of its own: a failed connect returns its error, and
`RetryTrigger::ConnectionError` decides whether the request runs again.

These cases send a request again, or connect again, without asking the policy.
Each happens at most once for one attempt, and the result then goes to
`RetryPolicy` like any other:

- HTTP/1.1: a pooled keep-alive connection fails before you receive the
  response. The pool sends an idempotent request with a buffered or empty body
  once more on a new connection.
- HTTP/2: a pooled connection fails before you receive the response. The pool
  sends a request with a buffered or empty body once more on a new connection
  when the method is idempotent. After a `REFUSED_STREAM` reset, it does so for
  any method, because the server refused the stream before it processed the
  request.
- HTTP/3: a pooled connection reports that it did not send the request, for
  example after a `GOAWAY` frame. The pool sends a request with a buffered or
  empty body once more on a new connection, for any method. After an
  `H3_REQUEST_REJECTED` reset that arrives before the response head, the
  connection sends a buffered request once more on the same connection.
- ALPN fallback: under `ProtocolPolicy::Auto`, an origin that does not select
  `h2` gets the request again over HTTP/1.1 on a new connection, and Leyline
  remembers that the origin speaks HTTP/1.1 only. A streaming request body
  cannot be sent twice, so that request fails with `Kind::Request`.
- `Race` fallback: under `ProtocolPolicy::Race`, when both the HTTP/3 connect
  and the HTTP/2 connect fail, Leyline falls back to `Auto` and connects
  again.

Backoff for attempt `n` is `initial_backoff * backoff_factor.powi(n)`, capped
at `max_backoff`. With `jitter` set, the result is multiplied by a uniform
random factor between 0 and 1, which is AWS-style full jitter.

## Retry-After wins

When a retryable response carries a `Retry-After` header, that value replaces
the computed backoff. Leyline parses both forms: delta-seconds, and an
IMF-fixdate with a `GMT` or `UTC` zone. An unparsable value falls back to the
computed backoff.

By default Leyline waits as long as `Retry-After` asks. To cap the wait, call
`max_retry_after`. If `Retry-After` then asks for longer than the cap,
Leyline stops retrying and returns the response, as it does when retries run
out. The caller can then fall back. Leyline also returns the response, or the
last error, when the wait is longer than the time left before the `total`
timeout.

## The idempotency rule

Leyline retries only idempotent methods by default: GET, HEAD, OPTIONS, PUT,
DELETE, and TRACE, per RFC 9110 section 9.2.2. A POST or PATCH is sent once,
whatever the policy says.

Opt in on the policy with `allow_non_idempotent(true)` when you know the
endpoint is safe to repeat.

```rust,no_run
use leyline::RetryPolicy;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session
    .post("https://example.com/idempotent-write")
    .json(&serde_json::json!({ "id": "fixed-key" }))
    .retry(RetryPolicy::transient().allow_non_idempotent(true))
    .await?;
# let _ = resp;
# Ok(())
# }
```

A streaming request body is never retried, whatever the method. See
[Streaming](streaming.md).

## The four timeouts

`TimeoutConfig` holds all of them. Each setter takes a `Duration` or `None`;
`None` turns that timeout off.

| Setter | Default | Covers |
| --- | --- | --- |
| `total` | `Some(300 s)` | Wall clock for one `send`, covering every redirect, retry, backoff sleep, and buffered body read. On expiry the call returns `Kind::Timeout`. `total(None)` removes the limit, for example for a long poll. |
| `connect` | `Some(10 s)` | DNS, TCP connect, proxy negotiation, and TLS setup for one new connection, over `http` or `https`. An HTTP/3 connection counts the DNS lookup, the UDP setup, and the QUIC handshake. Each new connection gets its own window, so a retry, a pool resend, the ALPN fallback, and the `Race` fallback each start a full window, and `total` bounds the sum. Pooled reuse is not covered. |
| `read` | `None` | Idle limit for each body read of a `.stream()` response: the longest gap between two chunks. A buffered response has no `read` limit, because its whole body arrives before the response resolves, and `response_header` and `total` cover that wait. A streamed body has only `read`. |
| `response_header` | `None` | Wait from dispatch start until the transport response resolves, per redirect: connection acquisition, DNS and TLS setup, and request transmission are inside this window. On a buffered response, the transport resolves after the whole body has arrived, so the body read is inside this window too. |

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
            .response_header(Duration::from_secs(20)),
    )
    .build()?;
# let _ = session;
# Ok(())
# }
```

`SessionBuilder::timeout` takes a `Duration` or a `TimeoutConfig`. A
`Duration` becomes `TimeoutConfig::new().total(duration)`, so the other
settings keep their defaults. A later `timeout` call replaces the whole
configuration. To change `connect` alone, pass
`TimeoutConfig::new().connect(duration)`.

## Per-request timeouts

`RequestBuilder::timeout` merges over the session timeouts, setting by
setting. A setting that the request sets wins, including `None`, which turns
it off for that request. Every setting the request does not touch keeps the
session value. A `Duration` sets only `total`. `connect` stays session-wide,
because connections are pooled and coalesced across requests; a request value
for it has no effect.

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

`Error::is_timeout()` returns true for every timeout, but the error kind
depends on which timeout elapsed:

- `total` and `response_header` return `Kind::Timeout`.
- `connect` returns `Kind::Connect` with a `TimedOut` I/O source, on HTTP/1.1,
  HTTP/2, and HTTP/3.
- A `read` stall on a `.stream()` body returns `Kind::Timeout` from `bytes()`,
  `text()`, and `json()`. It returns `Kind::Io` with a `TimedOut` I/O source
  from `copy_to` and `read_until`.

## Next

Read [Errors](errors.md) to match on `Kind` and find out which errors a
retry covers.
