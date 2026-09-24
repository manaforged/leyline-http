# Errors

Every call returns `leyline::Result<T>`, and every failure is a
`leyline::Error`. `err.kind()` returns a `Kind`. The [API map](../api.md#errors)
lists what each `Kind` means.

## Match on the kind

`Kind` is `#[non_exhaustive]`, so a `match` needs a wildcard arm.

```rust,no_run
use leyline::{Kind, Session};

# async fn run() {
let session = Session::new();
match session.get("https://example.com/").await {
    Ok(resp) => println!("{}", resp.status()),
    Err(err) => match err.kind() {
        Kind::Timeout => eprintln!("timed out: {err}"),
        Kind::Connect | Kind::Tls | Kind::Proxy => eprintln!("no connection: {err}"),
        Kind::Config | Kind::Url => eprintln!("fix the request: {err}"),
        _ => eprintln!("request failed: {err}"),
    },
}
# }
```

`Kind::as_str()` returns a stable lowercase label for logs and metrics.

`Config` means that a value or a combination of settings cannot work, for
example a forced HTTP/3 policy with a proxy, or a browser with no identity for
the platform. `Connect` means DNS resolution or the TCP connect failed.
`Proxy` means the proxy dial, handshake, authentication, or `CONNECT` failed;
`err.tls()` returns `TlsError::Proxy`, whose `status` holds the proxy's HTTP
status, for example `407`. `Tls` means the TLS handshake or certificate
verification failed.

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
| `is_retryable()` | `is_timeout()`, `is_connect()`, a reset, aborted, or closed connection, or a proxy dial failure |

For other kinds, compare `err.kind()`, for example `err.kind() == Kind::Redirect`.

## Which errors a retry covers

A `RetryPolicy` retries an error only in these cases:

- `is_timeout()` is true and the policy has `RetryTrigger::Timeout`.
- `is_retryable()` is true for another reason and the policy has
  `RetryTrigger::ConnectionError`.

A custom retry loop calls `err.is_retryable()` to use the same rule.

Other errors are not retried. `RetryPolicy::transient()` has both triggers. See [Retries and timeouts](retries-and-timeouts.md).

## Read the source error

`err.tls()` returns the `TlsError`, `err.h2()` returns the `H2Error`, and
`err.io()` returns the `std::io::Error`, when that is the source. Match
`TlsError::Rejected` for a peer that closed or reset the handshake, and
`TlsError::Certificate { verify_code, reason, .. }` for a failed certificate
check. `err.url()`
returns the request URL, and `without_url()` removes it before you log the
error.
