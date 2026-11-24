# Errors

Every fallible call returns `leyline::Result<T>` with error type
`leyline::Error`. Match on `err.kind()` (`Kind`, 19 variants) or one of the six
predicates rather than parsing error strings. Because `Kind` is
`#[non_exhaustive]`, always include a wildcard `_ =>` arm when matching.

`leyline::Error` implements `std::error::Error + Send + Sync + 'static`. So
`?` converts it into `Box<dyn std::error::Error + Send + Sync>` or
`anyhow::Error`.

`?` also converts `std::io::Error`, `url::ParseError` (`Kind::Url`), and
`http` header and URI errors into `leyline::Error`. It does not convert
`serde_json::Error`. To fail a hook with your own error, return
`Error::request(err)`.

## Match on the kind

When several kinds apply, `kind()` reports the first of these: a timeout, a
status error, a body limit, the network or TLS cause, then the component that
failed. `Kind::as_str()` and `Display` return a stable `snake_case` label for
logs, metrics, and API error responses.

| `Kind` | `as_str()` | When it happens |
| --- | --- | --- |
| `Timeout` | `"timeout"` | `total`, `connect`, `response_header`, `read`, or `body` deadline expired |
| `Dns` | `"dns"` | Name resolution failed (TCP, DoH, or HTTP/3) |
| `Connect` | `"connect"` | TCP connect failed, refused, or unreachable |
| `Tls` | `"tls"` | TLS handshake, certificate, hostname, or pin check failed (H1, H2, or H3) |
| `Proxy` | `"proxy"` | Proxy dial, handshake, auth, `CONNECT`, or invalid env proxy failed |
| `ProxyTarget` | `"proxy_target"` | Proxy worked but could not reach the origin |
| `Status` | `"status"` | `error_for_status()` (or `RequestBuilder::download_to`) rejected a `4xx`/`5xx` response |
| `BodyLimit` | `"body_limit"` | Response body passed `max_body_size` or a caller limit |
| `Body` | `"body"` | Failed or unreplayable request body stream, or WebSocket protocol or size error |
| `Decode` | `"decode"` | Decompression failed (corrupt data, truncated stream, or > 4 codings) |
| `Json` | `"json"` | JSON serialization or deserialization failed |
| `Http2` | `"http2"` | HTTP/2 protocol failure or forced H2 to an H1 server |
| `Http3` | `"http3"` | HTTP/3 / QUIC protocol failure |
| `Redirect` | `"redirect"` | Custom redirect policy > 32 hops, non-http(s) `Location`, or unreplayable body on `307`/`308` |
| `Url` | `"url"` | URL failed to parse or relative URL without `base_url` |
| `Config` | `"config"` | Invalid session/request/TLS/proxy/socket option, or `build()` rejected env proxy |
| `Request` | `"request"` | Malformed HTTP request, invalid header, `Session::shutdown()`, or `Error::request(...)` |
| `Io` | `"io"` | Socket I/O error after connect, malformed HTTP/1.1 wire response, or a file that `Body::file` cannot open or read |
| `Overloaded` | `"overloaded"` | `HostLimits::max_pending` or `max_total_pending` queue limit exceeded |

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

Formatting an `Error` with `{}` (`Display`) produces a single-line summary with
the kind, status, message, and redacted URL (passwords and query strings
hidden). Walk `std::error::Error::source()` for the underlying cause.

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
builder to turn a status of 400 or more into a `Kind::Status` error. The
check runs on the final response, after the retry policy is done. The error
keeps the start of the decoded body:

| Limit | Default | Set with |
| --- | --- | --- |
| Body bytes kept, after decoding | 64 KiB, and never more than `max_body_size` | `SessionBuilder::max_error_body_size` |
| Time to read that body | 10 s, and never past the `total` timeout | `Timeout::error_body` |

- Leyline applies both limits as soon as it sees the error status. Decompression stops at the byte limit, so a large error page cannot exhaust memory.
- If the read fails or times out, the error keeps the status, the headers, and the bytes that arrived. Leyline drops `content-encoding` when it decoded the body, and `content-length` when the body was cut short.
- On HTTP/2 and HTTP/3, an error body over the limit resets only that stream.
- On HTTP/1.1, the rest of a fixed-length error body up to `max_error_body_size` drains in the background, so the connection stays reusable. A larger or unframed error body closes the connection.

| Method on `Error` | Returns |
| --- | --- |
| `status()` | `Option<StatusCode>` |
| `url()` | `Option<&Url>` (final URL after redirects; set on `Kind::Status` only) |
| `headers()` | `Option<&HeaderMap>` (set on `Kind::Status` only) |
| `body()` | `Option<&[u8]>` (the captured decoded body prefix; `None` when no byte arrived) |
| `retry_after()` | `Option<Duration>` (wait read from `wait_header` or `Retry-After`; `None` for non-status errors) |
| `retries_exhausted()` | `bool` (`true` when the retry policy stopped on this status because of the attempt count, the retry budget, `max_wait`, or the remaining `total` time; set only by `RequestBuilder::error_for_status()`) |

For a response you already hold, `Response::error_for_status()` returns the
same `Kind::Status` error with the status, URL, and headers. It captures no
body. It borrows the response, so you can still read the body; see
[Responses](responses.md#turn-a-status-into-an-error).

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

Use `Error::request(source: impl Into<Box<dyn std::error::Error + Send + Sync>>)`
inside hooks such as `SessionBuilder::before_send` or `refresh_token` to abort
a request with non-retryable `Kind::Request`:

```rust
let err = leyline::Error::request("signing key missing");
assert_eq!(err.kind(), leyline::Kind::Request);
```

## Map errors to HTTP statuses

Backend services and reverse proxies calling an upstream through Leyline can
map failures to client-facing HTTP status codes with `Kind::gateway_status()`:

| `Kind` | `gateway_status()` | Reason |
| --- | --- | --- |
| `Status` | `None` | Pass the upstream `err.status()` through |
| `Timeout` | `504 Gateway Timeout` | Upstream did not finish in time |
| `Overloaded` | `503 Service Unavailable` | Local queue cap (`HostLimits::max_pending` / `max_total_pending`) exceeded |
| `Url`, `Config`, `Request` | `500 Internal Server Error` | Local configuration or request error (check `err.is_shut_down()` first for `503`) |
| Every other kind | `502 Bad Gateway` | Upstream or proxy failure (including `BodyLimit`, since upstream exceeded your limit) |

```rust,no_run
use leyline::http::StatusCode;

fn to_status(err: &leyline::Error) -> StatusCode {
    if err.is_shut_down() {
        return StatusCode::SERVICE_UNAVAILABLE;
    }
    err.kind()
        .gateway_status()
        .or_else(|| err.status())
        .unwrap_or(StatusCode::BAD_GATEWAY)
}
# let _ = to_status;
```

Log `Proxy` and `ProxyTarget` separately from upstream failures. See
[Service integration](service-integration.md).

## Read protocol detail and unwrap wrapped errors

- `Error::proxy_status()` returns the proxy's HTTP `CONNECT` status (`407`, `502`, ...) or SOCKS5 reply code.
- `Error::h2_code()` returns the HTTP/2 `RST_STREAM` or `GOAWAY` error code (for example `7` for `REFUSED_STREAM`).
- `Error::find(err)` walks an error's `source()` chain and returns the first `leyline::Error` in it. It also looks inside a `std::io::Error` or a Tower `BoxError`.

```rust,no_run
use std::error::Error as _;

fn reset_by_peer(err: &leyline::Error) -> bool {
    err.source()
        .and_then(|s| s.downcast_ref::<std::io::Error>())
        .is_some_and(|io| io.kind() == std::io::ErrorKind::ConnectionReset)
}
# let _ = reset_by_peer;
```
