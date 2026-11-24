# Errors

Every fallible call returns `leyline::Result<T>` with error type
`leyline::Error`. Match on `err.kind()` (`Kind`, 19 variants, `#[non_exhaustive]`)
or one of the six predicates rather than parsing error strings.

`leyline::Error` implements `std::error::Error + Send + Sync + 'static`, so
`?` converts it into `Box<dyn std::error::Error + Send + Sync>` or
`anyhow::Error`. It also converts from `std::io::Error` and `url::ParseError`
(`Kind::Url`).

## Match on the kind

`err.kind()` classifies each error in priority order: `Timeout` → `Status` →
`BodyLimit` → underlying transport or TLS cause → failing layer. `Kind::as_str()`
(and `Display`) returns a stable `snake_case` label for logs, metrics, and
API error responses.

| `Kind` | `as_str()` | `gateway_status()` | When it happens |
| --- | --- | --- | --- |
| `Timeout` | `"timeout"` | `504` | `total`, `connect`, `response_header`, `read`, or `body` deadline expired |
| `Dns` | `"dns"` | `502` | Name resolution failed (TCP, DoH, or HTTP/3) |
| `Connect` | `"connect"` | `502` | TCP connect failed, refused, or unreachable |
| `Tls` | `"tls"` | `502` | TLS handshake, certificate, hostname, or pin check failed (H1, H2, or H3) |
| `Proxy` | `"proxy"` | `502` | Proxy dial, handshake, auth, `CONNECT`, or invalid env proxy failed |
| `ProxyTarget` | `"proxy_target"` | `502` | Proxy worked but could not reach the origin |
| `Status` | `"status"` | `None` | `error_for_status()` rejected a `4xx`/`5xx` response |
| `BodyLimit` | `"body_limit"` | `502` | Response body passed `max_body_size` or a caller limit |
| `Body` | `"body"` | `502` | Response body already taken, unreplayable/failed request stream, or WebSocket protocol/size error |
| `Decode` | `"decode"` | `502` | Decompression failed (corrupt data, truncated stream, or > 4 codings) |
| `Json` | `"json"` | `502` | JSON serialization or deserialization failed |
| `Http2` | `"http2"` | `502` | HTTP/2 protocol failure or forced H2 to an H1 server |
| `Http3` | `"http3"` | `502` | HTTP/3 / QUIC protocol failure |
| `Redirect` | `"redirect"` | `502` | Custom redirect policy > 32 hops, non-http(s) `Location`, or unreplayable body on 307/308 |
| `Url` | `"url"` | `500` | URL failed to parse or relative URL without `base_url` |
| `Config` | `"config"` | `500` | Invalid session/request/TLS/proxy/socket option, or `build()` rejected env proxy |
| `Request` | `"request"` | `500` | Malformed HTTP request, invalid header, `Session::shutdown()`, or `Error::request(...)` |
| `Io` | `"io"` | `502` | Socket I/O error after connect, or malformed HTTP/1.1 wire response |
| `Overloaded` | `"overloaded"` | `503` | `HostLimits::max_pending` or `max_total_pending` queue limit exceeded |

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

`Display` prints the kind label, status, message, and redacted URL once. Walk
`std::error::Error::source()` for the underlying cause.

## Attempts and the proxy used

`err.attempts()` returns the number of transport attempts (`0` for `build()`
errors). `err.proxy()` returns the redacted proxy URL of the last attempt
(`None` when direct). Both are populated for `Kind::Status` errors too.

```rust,no_run
# async fn run() {
let session = leyline::Session::new();
if let Err(err) = session.get("https://api.example/").error_for_status().send().await {
    eprintln!(
        "{} after {} attempts via {}: {err}",
        err.kind().as_str(),
        err.attempts(),
        err.proxy().map(leyline::Url::as_str).unwrap_or("direct"),
    );
}
# }
```

## Status codes are not errors

A `4xx` or `5xx` response is `Ok`. Call `error_for_status()` on the request
builder to convert statuses `>= 400` into `Kind::Status` after retries finish,
keeping up to `max_error_body_size` (64 KiB default) of decoded body (see
[Turn a status into an error](responses.md#turn-a-status-into-an-error)).

| Method on `Error` | Returns |
| --- | --- |
| `status()` | `Option<StatusCode>` |
| `url()` | `Option<&Url>` (final URL after redirects; only set on `Kind::Status`) |
| `headers()` | `Option<&HeaderMap>` (only set on `Kind::Status`) |
| `body()` | `Option<&[u8]>` (captured decoded body prefix, or `None` if reading failed) |
| `retry_after()` | `Option<Duration>` (wait read from `wait_header` / `Retry-After`; `None` for non-status errors) |
| `retries_exhausted()` | `bool` (`true` when retries stopped due to max attempts, retry budget, `max_wait`, or remaining `total` timeout) |

`Display` and `Debug` redact URL passwords and queries. For a response you
already hold, `Response::error_for_status()` borrows the response and returns
a `Kind::Status` error with status, URL, and headers (no captured body).

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

## Predicates

Six predicates answer what to do next:

| Predicate | True for |
| --- | --- |
| `is_timeout()` | Any `Kind::Timeout` error (including connect timeouts) |
| `is_connect()` | `Kind::Connect`, `Kind::Dns`, TLS handshake failures, or connect timeouts (`false` for cert/hostname/pin failures) |
| `is_proxy()` | The proxy itself failed before the tunnel opened, including proxy connect timeouts (`false` for `ProxyTarget` and origin errors). See [Decide whether to ban a proxy](proxies.md#decide-whether-to-ban-a-proxy) |
| `is_retryable()` | `Kind::Timeout`, `is_connect()`, reset/aborted/closed connections, proxy dial/I/O failures, proxy `CONNECT` `502`/`503`/`504`, or `Kind::Status` `429`/`502`/`503`/`504` |
| `is_shut_down()` | `Session::shutdown()` stopped the session |
| `is_profile_changed()` | `Session::load(..)?.build()` detected a changed `profile_id` (`Kind::Config`) |

The default retry rule retries what `is_retryable()` accepts. See
[Retries and timeouts](retries-and-timeouts.md#choose-what-to-retry).

## Build a request error

Use `Error::request(cause)` inside hooks such as `SessionBuilder::before_send`
or `refresh_token` to abort a request with `Kind::Request` (non-retryable):

```rust
let err = leyline::Error::request("signing key missing");
assert_eq!(err.kind(), leyline::Kind::Request);
```

## Map errors to HTTP statuses

A service calling an upstream through Leyline can map failures directly with
`Kind::gateway_status()` (see the table above: `None` for `Status`, `504` for
`Timeout`, `503` for `Overloaded`, `500` for `Url`/`Config`/`Request`, and
`502` for everything else):

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

`BodyLimit` maps to `502` (not `413`) because the upstream response exceeded
your limit. Log `Proxy` and `ProxyTarget` separately from upstream failures.
See [Service integration](service-integration.md).

## Read protocol detail and unwrap wrapped errors

- `Error::proxy_status()` returns the proxy's HTTP `CONNECT` status (`407`, `502`, ...) or SOCKS5 reply code.
- `Error::h2_code()` returns the HTTP/2 `RST_STREAM` or `GOAWAY` error code (for example `7` for `REFUSED_STREAM`).
- `Error::find(err)` walks an error's `source()` chain — including errors wrapped inside a `std::io::Error` — and returns the first `leyline::Error` in it.

```rust,no_run
use std::error::Error as _;

fn reset_by_peer(err: &leyline::Error) -> bool {
    err.source()
        .and_then(|s| s.downcast_ref::<std::io::Error>())
        .is_some_and(|io| io.kind() == std::io::ErrorKind::ConnectionReset)
}
# let _ = reset_by_peer;
```

## Next

Read [Proxies](proxies.md) to set proxy rules, bypass lists, and environment
discovery.
