# Errors

Every call returns `leyline::Result<T>`, and every failure is a
`leyline::Error`. `err.kind()` sorts each error into one `Kind`.

`leyline::Error` implements `std::error::Error + Send + Sync + 'static`, so
`?` converts it into `Box<dyn std::error::Error + Send + Sync>` or
`anyhow::Error`. It also converts from `url::ParseError`, as `Kind::Url`.

## Match on the kind

`err.kind()` picks the first kind that applies, in this order:

| Order | Test | Kind |
| --- | --- | --- |
| 1 | The request, a body read, or a connect ran out of time | `Timeout`, a connect timeout included |
| 2 | `error_for_status` rejected the status | `Status` |
| 3 | A body passed `max_body_size` or a limit you gave | `BodyLimit` |
| 4 | The TLS-layer source error | `Dns`, `Connect` (TCP connect), `Proxy`, `ProxyTarget`, `Tls` (handshake, certificate, host name, pin), or `Config` (TLS setup) |
| 5 | The layer that failed | every other kind in the [`Kind` reference](https://docs.rs/leyline-http/latest/leyline/enum.Kind.html) |

`Kind` is `#[non_exhaustive]`, so a `match` needs a `_` arm.

```rust,no_run
use leyline::Kind;

# async fn run() {
let session = leyline::Session::new();
if let Err(err) = session.get("https://api.example/").send().await {
    match err.kind() {
        Kind::Timeout | Kind::Dns | Kind::Connect => {
            eprintln!("network: {err}")
        }
        Kind::Proxy | Kind::ProxyTarget => eprintln!("proxy: {err}"),
        Kind::Url | Kind::Config | Kind::Request => {
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
  unreachable. A connect that runs out of time is `Timeout`, and
  `is_connect()` is true for it.
- `Dns`: name resolution failed. A name resolution that runs out of time is
  `Timeout`. A session with `Protocol::Http3` reports a DNS failure as
  `Connect`.
- `Proxy`, `ProxyTarget`: the proxy failed, or it could not reach the origin.
  See [Proxy errors](proxies.md#proxy-errors).
- `Tls`: the TLS handshake or the certificate check failed. A handshake
  failure can pass on the next try, so `is_connect()` and `is_retryable()`
  are true for it. A certificate, host name, or pin failure stays until the
  server or your trust settings change, so both are false.
- `Decode`: a failed content decode. `Json`: a JSON serialize or deserialize
  failure.
- `Io`: a socket failure after the connect, or an HTTP/1.1 response that
  breaks the wire format.

`Kind::as_str()` returns a stable snake_case label, such as `"timeout"`,
`"proxy_target"`, or `"body_limit"`, for logs, metrics, and client-facing
error bodies. `Display` of a `Kind` prints the same label.

`Display` of an `Error` prints the kind label, status, message, and URL once.
Walk `std::error::Error::source()` for the cause.

## Attempts and the proxy used

`err.attempts()` returns the number of attempts, retries included, and `0`
for an error that no request produced, such as a `build()` failure.
`err.proxy()` returns the proxy URL of the last attempt, with the password
redacted, or `None` for a direct connection. Both are set for status errors
too.

```rust,no_run
# async fn run() {
let session = leyline::Session::new();
if let Err(err) = session.get("https://api.example/").error_for_status().send().await {
    eprintln!(
        "{} after {} attempts via {}: {err}",
        err.kind().as_str(),
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
| `body()` | `Option<&[u8]>`, the start of the body, or `None` when the read failed. `String::from_utf8_lossy` turns it into text |
| `retry_after()` | `Option<Duration>`, the wait before the next try |
| `retries_exhausted()` | `bool`, `true` when the retry policy wanted another try and did not make it |

`retry_after()` returns the wait that the retry policy read, each
`wait_header` first and then `Retry-After`, when the error comes from the
request-builder `error_for_status()`. Otherwise it parses `Retry-After`, in
seconds or as an HTTP date. It is `None` for an error that is not a status
error. See [Retries and timeouts](retries-and-timeouts.md#retry-after-wins).

`retries_exhausted()` is `true` when the rule still wanted a retry of the last
response and it stopped: no retries were left, the server asked for a
wait longer than `max_wait`, or the wait or backoff was longer than the
time left before the `total` timeout.

Only a `Kind::Status` error carries a URL and headers. `Display` and `Debug`
show the URL with the password and the query hidden.

```rust,no_run
use leyline::http::StatusCode;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
match session.get("https://api.example/users/7").error_for_status().send().await {
    Ok(resp) => println!("found: {}", resp.text().await?),
    Err(e) if e.status() == Some(StatusCode::NOT_FOUND) => println!("no such user"),
    Err(e) => return Err(e),
}
# Ok(())
# }
```

### Turn a status into an error

For a response you already hold, `Response::error_for_status()` borrows the
response and returns an error with the status, URL, and headers, and no
body. See
[Responses](responses.md#turn-a-status-into-an-error).

## Predicates

`kind()` answers which layer failed. Four predicates answer what to do next:

| Predicate | True for |
| --- | --- |
| `is_connect()` | Kind `Connect` or `Dns`, a TLS handshake failure, or a timeout while connecting. A TLS certificate, host name, or pin failure is false |
| `is_proxy()` | The proxy itself failed, before the tunnel was up, a proxy timeout included. See [Decide whether to ban a proxy](proxies.md#decide-whether-to-ban-a-proxy) |
| `is_retryable()` | Kind `Timeout`, `is_connect()`, a reset, aborted, or closed connection, an I/O failure while dialing or talking to a proxy, or a proxy answer of 502, 503, or 504 to `CONNECT` |
| `is_shut_down()` | The session was shut down with `Session::shutdown()` |

The default retry rule retries what `is_retryable()` accepts. See
[Retries and timeouts](retries-and-timeouts.md#choose-what-to-retry).

## Map errors to HTTP statuses

A service that calls an upstream through Leyline must answer its own client.
`Kind::gateway_status()` gives a default answer:

| Kind | `gateway_status()` | Reason |
| --- | --- | --- |
| `Status` | `None` | Pass the upstream status through, from `err.status()` |
| `Timeout` | 504 Gateway Timeout | The upstream did not answer in time |
| `Url`, `Config`, `Request` | 500 Internal Server Error | The service built a bad request |
| Every other kind | 502 Bad Gateway | The upstream or the proxy failed |

```rust,no_run
use leyline::http::StatusCode;

fn to_status(err: &leyline::Error) -> StatusCode {
    err.kind()
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

## Read the protocol detail

`Error::proxy_status()` returns the proxy's answer: the `CONNECT` status, such
as `407` or `502`, or the SOCKS5 reply code. `Error::h2_code()` returns the
HTTP/2 `RST_STREAM` or `GOAWAY` error code, such as `7` for
`REFUSED_STREAM`. `std::error::Error::source()` returns the
`std::io::Error` behind an I/O failure:

```rust,no_run
use std::error::Error as _;

fn reset_by_peer(err: &leyline::Error) -> bool {
    err.source()
        .and_then(|s| s.downcast_ref::<std::io::Error>())
        .is_some_and(|io| io.kind() == std::io::ErrorKind::ConnectionReset)
}
# let _ = reset_by_peer;
```

`Error::find(error)` walks an error's `source()` chain and returns the first
`leyline::Error` in it, so an error that a tower layer or your own type wraps
still maps through `kind()`.

## Next

Read [Proxies](proxies.md) to set proxy rules, bypass lists, and environment
discovery.
