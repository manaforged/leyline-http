# Errors

Every call returns `leyline::Result<T>`, and every failure is a
`leyline::Error`. `err.kind()` returns a `Kind`. The [API map](../api.md#errors)
lists what each `Kind` means.

## Match on the kind

`Kind` is `#[non_exhaustive]`, so a `match` needs a wildcard arm. Test
`is_timeout()` before the kind, because a timeout has more than one kind:

- A connect timeout is `Kind::Connect`, on HTTP/1.1, HTTP/2, and HTTP/3.
- A `total` or `response_header` timeout is `Kind::Timeout`.
- A stalled read of a `.stream()` body is `Kind::Timeout` from `bytes()`,
  `text()`, and `json()`. It is `Kind::Io` from `copy_to` and `read_until`.

`is_timeout()` is true for all of them.

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

`Kind::as_str()` returns a stable lowercase label for logs and metrics.

`Config` means that a value or a combination of settings cannot work, for
example a forced HTTP/3 policy with a proxy that cannot carry HTTP/3, or a
browser with no identity for the platform. `SessionBuilder::build()` also
returns it when a proxy environment variable is not a valid proxy URL.

`Connect` means DNS resolution or the TCP connect failed, or the connect
timeout elapsed. The connect window also covers proxy negotiation and the TLS
handshake, and on HTTP/3 the QUIC handshake. An HTTP/3 handshake that fails
before the window ends is `Kind::Http3`.

`Proxy` means the proxy dial, handshake, authentication, or `CONNECT` failed;
`err.tls()` returns `TlsError::Proxy`, whose `status` holds the proxy's HTTP
status, for example `407`. A session from `Session::new()` fails every request
with `Kind::Proxy` when a proxy environment variable is not a valid proxy URL,
and the message names the variable. `Tls` means the TLS handshake or
certificate verification failed.

`Display` prints the kind, status, message, and URL once. Walk
`std::error::Error::source()` for the cause.

## Status codes are not errors

A `4xx` or `5xx` response is `Ok`. Call `error_for_status()` to turn it into an
error of kind `Kind::Status`. `err.status()` then returns the code.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session.get("https://example.com/").await?.error_for_status()?;
# let _ = resp;
# Ok(())
# }
```

## Predicates

The predicates look at the kind and at the source error:

| Predicate | True for |
| --- | --- |
| `is_timeout()` | `Kind::Timeout`, and I/O or TLS errors with `TimedOut` |
| `is_connect()` | `Kind::Connect`, DNS, TCP connect, and TLS handshake failures, and refused or unreachable sockets |
| `is_status()` | `Kind::Status` |
| `is_retryable()` | `is_timeout()`, `is_connect()`, a reset, aborted, or closed connection, an I/O failure while dialing or talking to a proxy, or a proxy answer of 502, 503, or 504 to `CONNECT` |

For other kinds, compare `err.kind()`, for example `err.kind() == Kind::Redirect`.

## Which errors a retry covers

A `RetryPolicy` retries an error only in these cases:

- `is_timeout()` is true and the policy has `RetryTrigger::Timeout`.
- `is_retryable()` is true for another reason and the policy has
  `RetryTrigger::ConnectionError`.

A proxy that answers `CONNECT` with 502, 503, or 504 gives an error, not a
response, so the `Status` and `ServerError` triggers do not match it. Only
`ConnectionError` retries it. Other `CONNECT` answers, for example 407, are not
retried.

A custom retry loop calls `err.is_retryable()` to use the same rule.

Other errors are not retried. `RetryPolicy::transient()` has both triggers. See
[Retries and timeouts](retries-and-timeouts.md).

## Read the source error

`err.tls()` returns the `TlsError`, `err.h2()` returns the `H2Error`, and
`err.io()` returns the `std::io::Error`, when that is the source. Match
`TlsError::Rejected` for a peer that closed or reset the handshake, and
`TlsError::Certificate { verify_code, reason, .. }` for a failed certificate
check.

Only a `Kind::Status` error carries a URL. `err.url()` returns the URL of the
response, after any redirects, as an `Option<&url::Url>`, and returns `None` for
every other kind. `Display` and `Debug` show it with the password and the query
hidden.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session.get("https://example.com/").await?;
if let Err(err) = resp.error_for_status() {
    if let Some(url) = err.url() {
        eprintln!("status error on {}", url.host_str().unwrap_or("?"));
    }
}
# Ok(())
# }
```

## Next

Read [Proxies](proxies.md) to set proxy rules, bypass lists, and environment
discovery.
