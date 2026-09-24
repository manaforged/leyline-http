# Errors

Every call returns `leyline::Result<T>`, and every failure is a
`leyline::Error`. `err.kind()` returns a `Kind`. The [API map](../api.md#errors)
lists what each `Kind` means.

## Match on the kind

`Kind` is `#[non_exhaustive]`, so a `match` needs a wildcard arm.

```rust,no_run
use leyline::{Kind, Session};

# async fn run() {
let session = Session::chrome();
match session.get("https://example.com/").await {
    Ok(resp) => println!("{}", resp.status()),
    Err(err) => match err.kind() {
        Kind::Timeout => eprintln!("timed out: {err}"),
        Kind::Connect | Kind::Tls | Kind::Proxy => eprintln!("no connection: {err}"),
        Kind::Builder | Kind::Config | Kind::Url => eprintln!("fix the request: {err}"),
        _ => eprintln!("request failed: {err}"),
    },
}
# }
```

`Kind::as_str()` returns a stable lowercase label for logs and metrics.

`Builder` and `Config` are both configuration errors. `Builder` means that
`build()` or `send()` rejected a value you passed. `Config` means that the
combination of settings cannot work, for example a forced HTTP/3 policy with
a proxy, or a browser with no identity for the platform.

## Status codes are not errors

A `4xx` or `5xx` response is `Ok`. Call `error_for_status()` to turn it into an
error of kind `Kind::Status`. `err.status()` then returns the code.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::chrome();
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
| `is_connection_closed()` | A reset, aborted, or closed connection, an HTTP/2 `GOAWAY` with no error, and a refused HTTP/2 stream |
| `is_status()`, `is_redirect()`, `is_body()`, `is_decode()` | The matching `Kind` |

## Which errors a retry covers

A `RetryPolicy` retries an error only in these cases:

- `is_timeout()` is true and the policy has `RetryTrigger::Timeout`.
- `is_connect()` or `is_connection_closed()` is true and the policy has
  `RetryTrigger::ConnectionError`.

Other errors are not retried. `RetryPolicy::transient()` has both triggers.
`TlsError::is_retryable()` answers the same question for a TLS error that you
get from `err.tls()`. See [Retries and timeouts](retries-and-timeouts.md).

## Read the source error

`err.tls()` returns the `TlsError`, `err.h2()` returns the `H2Error`, and
`err.io()` returns the `std::io::Error`, when that is the source. `err.url()`
returns the request URL, and `without_url()` removes it before you log the
error.
