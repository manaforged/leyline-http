# Logging and tracing

Leyline logs through the [`tracing`](https://docs.rs/tracing) crate. It
installs no subscriber. Your application installs one, for example
`tracing_subscriber::fmt`, and filters by target.

## Targets

Every target starts with `leyline`.

| Target | Events |
| --- | --- |
| `leyline::session` | Session setup, for example the notice that a session defaults to Windows |
| `leyline::env_proxy::cgi` | A `HTTP_PROXY` value that the CGI guard ignored |
| `leyline::pool` | Connection pool events |
| `leyline::socket` | Socket options that the platform does not support |
| `leyline::tls` | TLS setup |
| `leyline::tls::trust` | Loading and checking the trust store |
| `leyline::h2`, `leyline::h2::flood_guard` | HTTP/2 connection events and flood protection |
| `leyline::quic` | HTTP/3 and QUIC events |
| `leyline::profile` | Profile load warnings, for example a missing `captured_against` |
| `leyline::digest` | Digest authentication |
| `leyline::trace` | Request phases from `TracingTrace` |

With a subscriber that reads `RUST_LOG`, this filter shows the request phases
and the warnings from the rest of the crate:

```sh
RUST_LOG=leyline=warn,leyline::trace=debug cargo run
```

## Log each request phase

`leyline::trace::TracingTrace` writes each phase of every request as a
`tracing` event under the `leyline::trace` target: DNS, connect, TLS, send,
response head, and completion.

```rust,no_run
use leyline::trace::TracingTrace;
use leyline::{Browser, Session};

# fn run() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::default())
    .trace(TracingTrace)
    .build()?;
# let _ = session;
# Ok(())
# }
```

To handle the phases in your own code, implement `leyline::trace::Trace`. The
`sent` event carries the method and the path with the query, so a
per-request log line needs no other hook. See
[Trace the request lifecycle](sessions.md#trace-the-request-lifecycle).
