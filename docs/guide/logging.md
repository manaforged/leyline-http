# Logging and tracing

Leyline logs through the [`tracing`](https://docs.rs/tracing) crate and
installs no subscriber. Your application installs one, for example
`tracing_subscriber::fmt`, and filters by target. For one event per request,
or counters, give the session a `Trace` hook.

## Targets

Every target starts with `leyline`.

| Target | Events |
| --- | --- |
| `leyline::session` | Session setup, for example the notice that a session defaults to Windows |
| `leyline::env_proxy::cgi` | An `HTTP_PROXY` value that the CGI guard ignored |
| `leyline::pool` | Connection pool events |
| `leyline::socket` | A `SocketConfig` option that the platform does not support |
| `leyline::tcp` | A `TcpProfile` socket option that the operating system rejected, once per option |
| `leyline::tls::trust` | Loading and checking the trust store |
| `leyline::h2`, `leyline::h2::flood_guard` | HTTP/2 connection events and flood protection |
| `leyline::quic` | HTTP/3 and QUIC events |
| `leyline::profile` | Profile load warnings, for example a missing `captured_against` |
| `leyline::digest` | Digest authentication |
| `leyline::trace` | `TracingTrace`: one summary per request, the end of each streamed body, and the request phases |

With a subscriber that reads `RUST_LOG`, this filter shows one line per
request and the warnings from the rest of the crate. Use
`leyline::trace=debug` to add the request phases.

```sh
RUST_LOG=leyline=warn,leyline::trace=info cargo run
```

## Log each request

`leyline::trace::TracingTrace` writes one INFO event per request on the
`leyline::trace` target, with the message `request`, and each phase as a
DEBUG event: DNS, connect, TLS, send, response head, and completion.

```rust,no_run
use leyline::trace::TracingTrace;
use leyline::{Browser, Session};

# async fn run() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::default())
    .trace(TracingTrace)
    .build()?;
let resp = session
    .get("https://shop.example/item/1")
    .tag("listing")
    .send()
    .await?;
println!("{} via {}", resp.status(), resp.proxy().unwrap_or("direct"));
# Ok(())
# }
```

The INFO event has the fields `id`, `method`, `url`, `redirects`, `status`,
`version`, `attempts`, `elapsed_ms`, `streamed`, `tag`, `proxy`, `browser`,
and `outcome`. `tag`, `proxy`, and `browser` appear only when they have a
value. A failed request adds `kind` and `error`. The URL is redacted: the
password, or a user name without a password, becomes `***`, the whole query
becomes `***`, and the fragment is removed.

```text
INFO leyline::trace: request id=7 method="GET" url=https://***@example.com/a?*** redirects=1 status=200 version=Http2 attempts=1 elapsed_ms=84 streamed=false outcome="ok"
INFO leyline::trace: request id=8 method="GET" url=https://shop.example/item/1 redirects=0 status=200 version=Http2 attempts=2 elapsed_ms=310 streamed=false tag="listing" proxy="http://user:***@proxy-a.example:8080/" browser=Chrome154 outcome="ok"
```

`RequestBuilder::tag(tag)` names a request, for example by job, account, or
identity. The `proxy` field holds the proxy the request used, from the
request, the session, or a `ProxyPool`, with the password replaced by `***`.
`Response::proxy()` returns the same value for your own log lines.

## One event per request

`Trace::summary` receives a `trace::Summary` once per request, after all
redirects and retries.

| Field | Value |
| --- | --- |
| `id` | The request id, the same as in the phase events |
| `method` | The request method |
| `url` | The final URL. On an error, the parsed original URL, or `None` when it does not parse |
| `original_url` | The URL as the caller gave it |
| `redirects` | The number of redirects followed |
| `status` | The final status, `None` on an error |
| `version` | The protocol of the final response, `None` on an error |
| `attempts` | The number of attempts, retries included |
| `elapsed` | The time from the start of the request |
| `outcome` | `Ok(())`, or `Err(&Error)` |
| `streamed` | `true` for a `.stream()` request |
| `tag` | The tag from `RequestBuilder::tag`, or `None` |
| `proxy` | The proxy the request used, redacted, or `None` |
| `browser` | The `Browser` of the session that sent the request, or `None` on a plain session. With `ProxyPool::identified`, the browser of the proxy's identity session |

For a `.stream()` request, `summary` fires at the response head, so it does
not cover the body read. It does not fire for a dropped request future. The
`done` phase event fires once per attempt, so a request with two retries
gives three `done` events and one `summary`.

`url` and `original_url` hold the raw URLs, with credentials and query
values. Redact them with `leyline::redact_url`, which `TracingTrace` uses,
before you log them.

```rust,no_run
use leyline::trace::{Summary, Trace};

struct RequestLog;

impl Trace for RequestLog {
    fn summary(&self, ev: &Summary<'_>) {
        let host = ev.url.and_then(|url| url.host_str()).unwrap_or("-");
        let status = ev.status.map_or(0, |status| status.as_u16());
        let tag = ev.tag.unwrap_or("-");
        let proxy = ev.proxy.as_deref().unwrap_or("direct");
        println!(
            "{} {} {} attempts={} {} ms tag={} proxy={}",
            ev.method,
            host,
            status,
            ev.attempts,
            ev.elapsed.as_millis(),
            tag,
            proxy
        );
    }
}

# fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .browser(leyline::Browser::default())
    .trace(RequestLog)
    .build()?;
# let _ = session;
# Ok(())
# }
```

A session takes one hook. `trace::Fanout::new().with(a).with(b)` sends each
event to several, and `trace::Metrics` counts them. See
[Count requests and watch the queues](crawling.md#count-requests-and-watch-the-queues).

## Streamed bodies

`Trace::body` receives a `trace::BodyEnd` once for each `.stream()` response,
when its body ends. A buffered response has no `body` event, because its
`summary` already covers the body read.

| Field | Value |
| --- | --- |
| `id` | The `id` of the request's `Summary` |
| `bytes` | The body bytes that the caller received, after decompression |
| `elapsed` | The time from the start of the request, not from the head |
| `outcome` | `BodyOutcome::Complete`, `Failed(&io::Error)` when a read failed, or `Dropped` when the caller dropped the response or body stream before the end |

`BodyOutcome` is `#[non_exhaustive]`, so a `match` needs a `_` arm.
`TracingTrace` writes the event at INFO with the message `body` and the fields
`id`, `bytes`, `elapsed_ms`, and `outcome` (`complete`, `error`, or
`dropped`). An error adds `kind` and `error`.

```text
INFO leyline::trace: body id=9 bytes=1048576 elapsed_ms=930 outcome="complete"
```

To write one line per request, keep the head of each streamed request by
`id` and write the line when its `BodyEnd` arrives. Write other requests from
`summary`:

```rust,no_run
use std::collections::HashMap;
use std::sync::Mutex;

use leyline::trace::{BodyEnd, BodyOutcome, Summary, Trace};

#[derive(Default)]
struct OneLine {
    heads: Mutex<HashMap<u64, String>>,
}

impl Trace for OneLine {
    fn summary(&self, ev: &Summary<'_>) {
        let head = format!("{} {:?} {:?}", ev.method, ev.status, ev.browser);
        if ev.streamed && ev.outcome.is_ok() {
            self.heads.lock().unwrap().insert(ev.id, head);
        } else {
            println!("{} {head} {} ms", ev.id, ev.elapsed.as_millis());
        }
    }

    fn body(&self, ev: &BodyEnd<'_>) {
        let head = self.heads.lock().unwrap().remove(&ev.id).unwrap_or_default();
        let outcome = match ev.outcome {
            BodyOutcome::Complete => "complete".to_owned(),
            BodyOutcome::Failed(err) => format!("failed: {err}"),
            BodyOutcome::Dropped => "dropped".to_owned(),
            _ => "other".to_owned(),
        };
        println!("{} {head} {} bytes {} ms {outcome}", ev.id, ev.bytes, ev.elapsed.as_millis());
    }
}
# let _ = OneLine::default();
```

## Phase events

To handle the phases in your own code, implement the other `Trace` methods.
The `sent` event carries the method and the path with the query. The `head`
event carries the status and the response headers as received, before
decompression, as `&http::HeaderMap`. See
[Trace the request lifecycle](sessions.md#trace-the-request-lifecycle).

## Next

Read [Service integration](service-integration.md) to use a session inside a
web service.
