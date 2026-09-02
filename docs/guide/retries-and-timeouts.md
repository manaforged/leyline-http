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
let session = leyline::Session::chrome();
let resp = session
    .get("https://example.com/flaky")
    .retry(RetryPolicy::transient().with_max_retries(4))
    .await?;
# let _ = resp;
# Ok(())
# }
```

Set a session-wide default with `SessionBuilder::retry`. Every request inherits
it unless the request sets its own with `RequestBuilder::retry`.

## Custom triggers

`RetryTrigger` has four variants: `ConnectionError`, `Status(u16)`,
`ServerError` for any 5xx, and `Timeout`. Start from `none()` or `transient()`
and set one field per call.

```rust
use leyline::{RetryPolicy, RetryTrigger};
use std::time::Duration;

let policy = RetryPolicy::none()
    .with_max_retries(5)
    .with_backoff(Duration::from_millis(50), Duration::from_secs(2))
    .backoff_factor(1.5)
    .jitter(true)
    .retry_on([RetryTrigger::ServerError, RetryTrigger::ConnectionError]);
assert_eq!(policy.max_retries, 5);

let also = RetryPolicy::transient().on_status(408);
assert_eq!(also.max_retries, 3);
```

`with_max_retries` and `with_backoff` adjust an existing policy. `on_status`
adds a status code to the trigger set.

Backoff for attempt `n` is `initial_backoff * backoff_factor.powi(n)`, capped
at `max_backoff`. With `jitter` set, the result is multiplied by a uniform
random factor between 0 and 1, which is AWS-style full jitter.

## Retry-After wins

When a retryable response carries a `Retry-After` header, that value replaces
the computed backoff. Leyline parses both forms: delta-seconds, and an
IMF-fixdate with a `GMT` or `UTC` zone. An unparsable value falls back to the
computed backoff.

## The idempotency rule

Leyline retries only idempotent methods by default: GET, HEAD, OPTIONS, PUT,
DELETE, and TRACE, per RFC 9110 section 9.2.2. A POST or PATCH is sent once,
whatever the policy says.

Opt in per request when you know the endpoint is safe to repeat.

```rust,no_run
use leyline::RetryPolicy;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::chrome();
let resp = session
    .post("https://example.com/idempotent-write")
    .json(&serde_json::json!({ "id": "fixed-key" }))
    .retry(RetryPolicy::transient())
    .allow_non_idempotent_retry(true)
    .await?;
# let _ = resp;
# Ok(())
# }
```

A streaming request body is never retried, whatever the method. See
[Streaming](streaming.md).

## The four timeouts

`TimeoutConfig` holds all of them. It is the single source of truth: the
session's total request timeout is `timeouts.total`.

| Field | Default | Covers |
| --- | --- | --- |
| `total` | 300 s | Wall clock for one `send`, covering every redirect hop, retry, backoff sleep, and buffered body read. On expiry the call returns `Error::Timeout`. |
| `connect` | `Some(10 s)` | DNS, TCP connect, and TLS setup for one new `https` connection, fired before the request is written. Pooled reuse and plaintext `http` connects are not covered. |
| `read` | `None` | Idle gap between chunks of a streamed response body. It fires only on a request that called `stream`; a buffered body is read inside the `response_header` and `total` windows instead. |
| `response_header` | `None` | Wait from request sent until the transport response resolves, per redirect hop. A buffered response resolves only after its body is read. |

```rust,no_run
use leyline::{Session, TimeoutConfig};
use std::time::Duration;

# fn run() -> leyline::Result<()> {
let session = Session::builder()
    .timeouts(
        TimeoutConfig::default()
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

`timeouts()` replaces every value, including anything set earlier by `timeout`
or `connect_timeout`. Set it first, then adjust with the narrow methods if you
want to.

`SessionBuilder::timeout` sets `total` alone. `SessionBuilder::connect_timeout`
sets `connect` alone.

## Per-request timeouts

`RequestBuilder::timeout` overrides `total` for one request.
`RequestBuilder::timeouts` overrides `total`, `read`, and `response_header`
together. `connect` stays session-wide either way, because connections are
pooled and coalesced across requests.

```rust,no_run
use leyline::TimeoutConfig;
use std::time::Duration;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::chrome();

let quick = session
    .get("https://example.com/slow")
    .timeout(Duration::from_secs(5))
    .await?;

let streamed = session
    .get("https://example.com/feed")
    .stream()
    .timeouts(
        TimeoutConfig::default()
            .total(Duration::from_secs(120))
            .read(Duration::from_secs(10))
            .response_header(Duration::from_secs(15)),
    )
    .await?;

# let _ = (quick, streamed);
# Ok(())
# }
```

A timeout surfaces as `Error::Timeout`, and `Error::is_timeout()` returns true
for it and for an underlying `TimedOut` I/O error.

## Next

Read [Proxies](proxies.md).
