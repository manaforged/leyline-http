# Errors

Every call returns `leyline::Result<T>`, and every failure is a
`leyline::Error`. Start with `err.category()`, which sorts each error into one
`ErrorCategory`. Use `err.kind()` only for the finer detail of
[Fine-grained kinds](#fine-grained-kinds).

`leyline::Error` implements `std::error::Error + Send + Sync + 'static`, so
`?` converts it into `Box<dyn std::error::Error + Send + Sync>` or
`anyhow::Error`. It also converts from `url::ParseError`, as `Kind::Url`.

## Group errors by category

`err.category()` picks the first group that applies, in this order:

| Order | Test | Category |
| --- | --- | --- |
| 1 | `is_timeout()` | `Timeout`, a connect timeout included |
| 2 | `is_status()` | `Status` |
| 3 | `is_body_limit()` | `BodyLimit` |
| 4 | The TLS-layer cause in `err.tls()` | `Dns`, `Connect` (TCP connect), `Proxy`, `ProxyTarget`, `Tls` (handshake, certificate, host name, pin), or `Config` (TLS setup) |
| 5 | `err.kind()` | See [Fine-grained kinds](#fine-grained-kinds) |

`ErrorCategory` is `#[non_exhaustive]`, so a `match` needs a `_` arm.

```rust,no_run
use leyline::ErrorCategory;

# async fn run() {
let session = leyline::Session::new();
if let Err(err) = session.get("https://api.example/").await {
    match err.category() {
        ErrorCategory::Timeout | ErrorCategory::Dns | ErrorCategory::Connect => {
            eprintln!("network: {err}")
        }
        ErrorCategory::Proxy | ErrorCategory::ProxyTarget => eprintln!("proxy: {err}"),
        ErrorCategory::Url | ErrorCategory::Config | ErrorCategory::Request => {
            eprintln!("fix the request: {err}")
        }
        _ => eprintln!("request failed: {err}"),
    }
}
# }
```

- `Config`: a value or a combination of settings cannot work, for example a
  forced HTTP/3 policy with a proxy that cannot carry HTTP/3, or a browser
  with no identity for the platform. `SessionBuilder::build()` also returns it
  when a proxy environment variable is not a valid proxy URL.
- `Connect`: the TCP connect failed, or the socket was refused or
  unreachable. The connect window also covers proxy negotiation, the TLS
  handshake, and on HTTP/3 the QUIC handshake.
- `Dns`: name resolution failed, and `is_dns()` is true.
- `Proxy`, `ProxyTarget`: the proxy failed, or it could not reach the origin.
  See [Proxy errors](proxies.md#proxy-errors).
- `Tls`: the TLS handshake or the certificate check failed. A handshake
  failure can pass on the next try, so `is_connect()` and `is_retryable()`
  are true for it. A certificate, host name, or pin failure stays until the
  server or your trust settings change, so both are false.
- `Decode`: a failed content decode, or a `Response::json` serde failure
  (`Kind::Json`).

`ErrorCategory::as_str()` returns a stable snake_case label, such as
`"timeout"`, `"proxy_target"`, or `"body_limit"`, for logs, metrics, and
client-facing error bodies. `Display` prints the same label.

`Display` of an `Error` prints the kind, status, message, and URL once. Walk
`std::error::Error::source()` for the cause.

## Attempts and the proxy used

`err.attempts()` returns the number of attempts, retries included, and `0`
for an error that no request produced, such as a `build()` failure.
`err.proxy()` returns the proxy URL of the last attempt, with the password
redacted, or `None` for a direct connection. Both are set for status errors
too.

```rust,no_run
# async fn run() {
let session = leyline::Session::new();
if let Err(err) = session.get("https://api.example/").error_for_status().await {
    eprintln!(
        "{} after {} attempts via {}: {err}",
        err.category().as_str(),
        err.attempts(),
        err.proxy().unwrap_or("direct"),
    );
}
# }
```

## Status codes are not errors

A `4xx` or `5xx` response is `Ok`. To get a status of 400 or more as an
error, call `error_for_status()` on the request builder. A failed send and a
bad status then arrive as one `Result`. The check runs after retries, so it
sees the last response, and the error keeps the start of the decoded body,
within the limits in
[Turn a status into an error](responses.md#turn-a-status-into-an-error).

| Method on the error | Returns |
| --- | --- |
| `status()` | `Option<StatusCode>`, the response status |
| `url()` | `Option<&Url>`, the final URL, after redirects |
| `headers()` | `Option<&HeaderMap>`, the response headers |
| `header(name)` | `Option<&str>`, one header as text. `None` when it is missing or not ASCII |
| `body()` | `Option<&[u8]>`, the start of the body, or `None` when the read failed |
| `body_text()` | `Option<Cow<str>>`, the kept body as text, with invalid UTF-8 replaced |
| `retry_after()` | `Option<Duration>`, the wait before the next try |
| `retries_exhausted()` | `bool`, `true` when the retry policy wanted another try and did not make it |

`retry_after()` returns the wait that the retry policy read, each
`wait_header` first and then `Retry-After`, when the error comes from the
request-builder `error_for_status()`. Otherwise it parses `Retry-After`, in
seconds or as an HTTP date. It is `None` for an error that is not a status
error. See [Retries and timeouts](retries-and-timeouts.md#retry-after-wins).

`retries_exhausted()` is `true` when the last response still matched a retry
trigger and the policy stopped: no retries were left, the server asked for a
wait longer than `max_retry_after`, or the wait or backoff was longer than the
time left before the `total` timeout.

Only a `Kind::Status` error carries a URL and headers. `Display` and `Debug`
show the URL with the password and the query hidden.

```rust,no_run
use leyline::http::StatusCode;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
match session.get("https://api.example/users/7").error_for_status().await {
    Ok(resp) => println!("found: {}", resp.text().await?),
    Err(e) if e.status() == Some(StatusCode::NOT_FOUND) => println!("no such user"),
    Err(e) => return Err(e),
}
# Ok(())
# }
```

### Turn a status into an error

For a response you already hold, `Response::error_for_status()` consumes the
response and `error_for_status_ref()` borrows it. Both errors keep the
status, URL, and headers, and no body. See
[Responses](responses.md#turn-a-status-into-an-error).

## Predicates

The predicates look at the kind and at the source error:

| Predicate | True for |
| --- | --- |
| `is_timeout()` | `Kind::Timeout`, and I/O or TLS errors with `TimedOut` |
| `is_connect()` | An error in category `Connect` or `Dns`, a TLS handshake failure, or a timeout while connecting. It agrees with `category()`: a TLS certificate, host name, or pin failure is false |
| `is_dns()` | Name resolution failed. A session with `ProtocolPolicy::Http3` reports a DNS failure as a connect failure |
| `is_status()` | `Kind::Status` |
| `is_body_limit()` | A response body passed `max_body_size` or a limit you gave. The message names the limit. False for a body already taken, a failed request body stream, and corrupt compressed data |
| `is_proxy()` | The proxy itself failed, before the tunnel was up. See [Decide whether to ban a proxy](proxies.md#decide-whether-to-ban-a-proxy) |
| `is_retryable()` | `is_timeout()`, `is_connect()`, a reset, aborted, or closed connection, an I/O failure while dialing or talking to a proxy, or a proxy answer of 502, 503, or 504 to `CONNECT` |
| `is_shut_down()` | The session was shut down with `Session::shutdown()` |

A `RetryPolicy` uses `is_timeout()` and `is_retryable()` to pick its
triggers. See [Retries and timeouts](retries-and-timeouts.md#custom-triggers).

## Map errors to HTTP statuses

A service that calls an upstream through Leyline must answer its own client.
`ErrorCategory::gateway_status()` gives a default answer:

| Category | `gateway_status()` | Reason |
| --- | --- | --- |
| `Status` | `None` | Pass the upstream status through, from `err.status()` |
| `Timeout` | 504 Gateway Timeout | The upstream did not answer in time |
| `Url`, `Config`, `Request` | 500 Internal Server Error | The service built a bad request |
| Every other category | 502 Bad Gateway | The upstream or the proxy failed |

```rust,no_run
use leyline::http::StatusCode;

fn to_status(err: &leyline::Error) -> StatusCode {
    err.category()
        .gateway_status()
        .or_else(|| err.status())
        .unwrap_or(StatusCode::BAD_GATEWAY)
}
# let _ = to_status;
```

`BodyLimit` maps to 502, not to 413: the upstream response passed the limit,
not the client's request. Log `Proxy` and `ProxyTarget` apart from upstream
failures, because they point at your proxy. See
[Service integration](service-integration.md) for a full service.

## Fine-grained kinds

`err.kind()` returns a `Kind`, the layer that failed. The
[API map](../api.md#errors) lists each `Kind`. `Kind` is `#[non_exhaustive]`,
and `Kind::as_str()` returns a stable lowercase label.

Because `category()` tests the timeout, the status, the body limit, and the
TLS-layer cause first, one kind can give more than one category:

| Kind | Category | Other category when |
| --- | --- | --- |
| `Timeout` | `Timeout` | |
| `Connect` | `Connect` | `Timeout` for a connect timeout, `Dns` for a failed name resolution |
| `Tls` | `Tls` | `Config` for a TLS setup or trust store failure |
| `Proxy` | `Proxy` | `ProxyTarget` when the proxy could not reach the origin |
| `Status` | `Status` | |
| `Body` | `Body` | `BodyLimit` for a body over a limit |
| `Decode`, `Json` | `Decode` | |
| `Http2`, `Http3` | `Protocol` | |
| `Redirect` | `Redirect` | |
| `Url` | `Url` | |
| `Config` | `Config` | |
| `Request` | `Request` | |
| `Io` | `Other` | `Connect` for a refused or unreachable socket, `BodyLimit` for a body over a limit |

Any kind whose I/O cause is `TimedOut` gives `Timeout`. A timeout has more
than one kind:

- A `connect` timeout is `Kind::Connect`, on HTTP/1.1, HTTP/2, and HTTP/3.
- A `total` or `response_header` timeout is `Kind::Timeout`.
- A `read` stall on a `.stream()` body is `Kind::Timeout` from `bytes()`,
  `text()`, and `json()`, and `Kind::Io` from `copy_to` and `read_until`.

So when you match on the kind, test `is_timeout()` first:

```rust,no_run
use leyline::{Kind, Session};

# async fn run() {
let session = Session::new();
match session.get("https://example.com/").await {
    Ok(resp) => println!("{}", resp.status()),
    Err(err) if err.is_timeout() => eprintln!("timed out: {err}"),
    Err(err) => match err.kind() {
        Kind::Connect | Kind::Tls | Kind::Proxy => eprintln!("no connection: {err}"),
        Kind::Config | Kind::Url => eprintln!("fix the request: {err}"),
        _ => eprintln!("request failed: {err}"),
    },
}
# }
```

## Read the source error

`err.tls()` returns the `TlsError`, `err.h2()` the `H2Error`, and `err.io()`
the `std::io::Error`, when that is the source. Match `TlsError::Rejected` for
a peer that closed or reset the handshake, and
`TlsError::Certificate { verify_code, reason, .. }` for a failed certificate
check. `TlsError::Proxy` and `TlsError::ProxyTargetUnreachable` are described
in [Proxy errors](proxies.md#proxy-errors).

`Error::find(error)` walks an error's `source()` chain and returns the first
`leyline::Error` in it, so an error that a tower layer or your own type wraps
still maps through `category()`.

## Next

Read [Proxies](proxies.md) to set proxy rules, bypass lists, and environment
discovery.
