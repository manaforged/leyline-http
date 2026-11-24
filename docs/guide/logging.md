# Logging and tracing

Leyline logs through the [`tracing`](https://docs.rs/tracing) crate and
installs no subscriber. Your application installs one, for example
`tracing_subscriber::fmt`, and filters by target. For one event per request,
or counters, give the session a `Trace` hook.

## Targets

Every target starts with `leyline`.

| Target | Events |
| --- | --- |
| `leyline::session` | `Protocol::Race` HTTP/3 fallback and proxy-skip debug events |
| `leyline::env_proxy::cgi` | An `HTTP_PROXY` value skipped by the CGI httpoxy guard |
| `leyline::pool` | Connection pool events |
| `leyline::socket` | Unsupported `SessionBuilder` or `TcpProfile` socket option warnings |
| `leyline::tls::trust` | Loading and checking the trust store |
| `leyline::h2`, `leyline::h2::flood_guard` | HTTP/2 connection events and flood protection |
| `leyline::quic` | HTTP/3 and QUIC events |
| `leyline::profile` | Profile load warnings (such as a missing `captured_against`) |
| `leyline::digest` | Digest authentication |
| `leyline::autosave` | Background `Session::autosave` write failures |
| `leyline::crawl` | Crawler robots/sitemap/fetch diagnostics |
| `leyline::trace` | `Log`: one summary per request, streamed body completion, and request phases |
| `leyline::cache` | Responses reused from the session HTTP cache |

With a subscriber that reads `RUST_LOG`, this filter shows one line per
request and the warnings from the rest of the crate. Use
`leyline::trace=debug` to add the request phases.

```sh
RUST_LOG=leyline=warn,leyline::trace=info cargo run
```

## Log each request

`leyline::trace::Log` writes one INFO event per request on the
`leyline::trace` target, with the message `request`, and each phase as a
DEBUG event: DNS, connect, TLS, send, response head, and completion.

```rust,no_run
use leyline::trace::Log;
use leyline::{Browser, Session};

# async fn run() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::default())
    .trace(Log)
    .build()?;
let resp = session
    .get("https://shop.example/item/1")
    .label("listing")
    .send()
    .await?;
println!("{} via {}", resp.status(), resp.proxy().map(leyline::Url::as_str).unwrap_or("direct"));
# Ok(())
# }
```

The INFO event has the fields `id`, `method`, `url`, `redirects`, `status`,
`version`, `attempts`, `elapsed_ms`, `streamed`, `label`, `proxy`, `browser`,
and `outcome`. `label`, `proxy`, and `browser` appear only when they have a
value. A failed request adds `kind` and `error`. The URL is redacted: the
password, or a user name without a password, becomes `***`, the whole query
becomes `***`, and the fragment is removed.

```text
INFO leyline::trace: request id=7 method="GET" url=https://***@example.com/a?*** redirects=1 status=200 version=HTTP/2.0 attempts=1 elapsed_ms=84 streamed=false outcome="ok"
INFO leyline::trace: request id=8 method="GET" url=https://shop.example/item/1 redirects=0 status=200 version=HTTP/2.0 attempts=2 elapsed_ms=310 streamed=false label="listing" proxy="http://user:***@proxy-a.example:8080/" browser=Chrome158 outcome="ok"
```

`RequestBuilder::label(text)` names a request, for example by job, account, or
identity. The `proxy` field holds the proxy the request used, from the
request, the session, or a proxy list, with the password replaced by `***`.
`Response::proxy()` returns the same value for your own log lines.

## One event per request

`Trace::event` receives an `Event::Summary` once per request, after all
redirects and retries.

| Field | Value |
| --- | --- |
| `id` | The request id: one per call to `send`, kept across redirects and retries. The phase events carry a separate id per attempt |
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
| `label` | The label from `RequestBuilder::label`, or `None` |
| `proxy` | The proxy the request used, redacted, or `None` |
| `browser` | The `Browser` of the session that sent the request, or `None` on a plain session. With `Proxy::identified`, the browser of the proxy's session |

For a `.stream()` request, `Summary` fires at the response head, so it does
not cover the body read. It does not fire for a dropped request future. The
`Done` phase event fires once per attempt, so a request with two retries
gives three `Done` events and one `Summary`.

`url` and `original_url` hold the raw URLs, with credentials and query
values. Redact them with `leyline::redact_url`, which `Log` uses,
before you log them.

```rust,no_run
use leyline::trace::{Event, Trace};

struct RequestLog;

impl Trace for RequestLog {
    fn event(&self, event: &Event<'_>) {
        let Event::Summary { method, url, status, attempts, elapsed, label, proxy, .. } = event
        else {
            return;
        };
        let host = url.and_then(|url| url.host_str()).unwrap_or("-");
        let status = status.map_or(0, |status| status.as_u16());
        let label = label.unwrap_or("-");
        let proxy = proxy.as_deref().unwrap_or("direct");
        println!(
            "{method} {host} {status} attempts={attempts} {} ms label={label} proxy={proxy}",
            elapsed.as_millis()
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

Each `trace` call adds a hook, and every hook sees every event.

## Count requests and inspect queues

`leyline::trace::Metrics` is a bundled `Trace` hook that counts session
activity. Wrap `Metrics::new()` in an `Arc`, pass a clone to `.trace(..)`, and
read counters from the handle (`Display` formats a single `key=value` line):

| Method | Signature / Count |
| --- | --- |
| `requests()`, `attempts()` | `u64` monotonic totals (attempts include retries) |
| `status_class(class: u8)` | `u64` responses in `class`xx (`2`, `3`, `4`, `5` — note `u8`, not `u16`) |
| `errors(kind)`, `errors_total()` | `u64` errors per `Kind` and all errors |
| `bodies_complete()`, `bodies_failed()`, `bodies_dropped()` | `u64` streamed body outcomes |
| `latency()` | `Vec<(Option<Duration>, u64)>` **per-bucket (non-cumulative)** counts of `Event::Summary` elapsed time (full request when buffered; time-to-headers when `.stream()`), with upper bounds 10, 50, 100, 250, 500, 1000, 2500, 5000, 10000 ms and `None` (> 10 s) |

Pair `Metrics` with `Session::stats()` for live connection and queue counts:
`hosts()` lists tracked origin strings, `in_flight(url)` and `waiting(url)`
accept `impl IntoUrl` (for example `"https://api.example/"` or an entry from
`stats.hosts()`; origins paused by `HostLimits::pause_on` count as waiting),
and `busy()` / `idle()` count connections.

```rust,no_run
use std::sync::Arc;
use std::time::Duration;

use leyline::{Kind, Session};
use leyline::trace::Metrics;

# async fn run() -> leyline::Result<()> {
let metrics = Arc::new(Metrics::new());
let session = Session::builder().trace(Arc::clone(&metrics)).build()?;
let _ = session.get("https://api.example/").send().await;
println!("{metrics} (2xx={})", metrics.status_class(2));
if metrics.errors(Kind::Proxy) > 100 {
    eprintln!("high proxy error rate");
}
let stats = session.stats();
for host in stats.hosts() {
    println!("{host} in_flight={} waiting={}", stats.in_flight(host), stats.waiting(host));
}
let _ = Duration::from_secs(10);
# Ok(())
# }
```

## Streamed bodies

`Event::Body` fires once for each `.stream()` response, when its body ends.
A buffered response has no `Body` event, because its `Summary` already covers
the body read.

| Field | Value |
| --- | --- |
| `id` | The `id` of the request's `Summary` |
| `bytes` | The body bytes that the caller received, after decompression |
| `elapsed` | The time from the start of the request, not from the head |
| `outcome` | `Ok(())`, `Err(Some(&io::Error))` when a read failed, or `Err(None)` when the caller dropped the response or body stream before the end |

`Log` writes the event at INFO with the message `body` and the fields
`id`, `bytes`, `elapsed_ms`, and `outcome` (`complete`, `error`, or
`dropped`). An error adds `kind` and `error`.

```text
INFO leyline::trace: body id=9 bytes=1048576 elapsed_ms=930 outcome="complete"
```

To write one line per request, keep the head of each streamed request by
`id` and write the line when its `Body` event arrives. Write other requests
from `Summary`:

```rust,no_run
use std::collections::HashMap;
use std::sync::Mutex;

use leyline::trace::{Event, Trace};

#[derive(Default)]
struct OneLine {
    heads: Mutex<HashMap<u64, String>>,
}

impl Trace for OneLine {
    fn event(&self, event: &Event<'_>) {
        match event {
            Event::Summary { id, method, status, browser, streamed, outcome, elapsed, .. } => {
                let head = format!("{method} {status:?} {browser:?}");
                if *streamed && outcome.is_ok() {
                    self.heads.lock().unwrap().insert(*id, head);
                } else {
                    println!("{id} {head} {} ms", elapsed.as_millis());
                }
            }
            Event::Body { id, bytes, elapsed, outcome, .. } => {
                let head = self.heads.lock().unwrap().remove(id).unwrap_or_default();
                let outcome = match outcome {
                    Ok(()) => "complete".to_owned(),
                    Err(Some(err)) => format!("failed: {err}"),
                    Err(None) => "dropped".to_owned(),
                };
                println!("{id} {head} {bytes} bytes {} ms {outcome}", elapsed.as_millis());
            }
            _ => {}
        }
    }
}
# let _ = OneLine::default();
```

## Phase events

Match the six per-attempt `Event` variants in `Trace::event` (all `Event`
variants are `#[non_exhaustive]`, so include `..` in struct patterns). Note for
`tracing::Span` correlation: `Summary` and HTTP/1.1 events run on the caller's
task, while HTTP/2 and HTTP/3 connection-open events (`Dns`, `Connect`, `Tls`)
fire on the spawned connection task (keep listeners non-blocking).

| Variant | Key fields | When it fires |
| --- | --- | --- |
| `Dns` | `id`, `host`, `addrs`, `elapsed` | Name resolution completed for a TCP connect (proxy host when proxied) |
| `Connect` | `id`, `host`, `port`, `addr`, `reused`, `elapsed` | Socket connected or reused from pool (`addr` matches `Response::remote_addr()`) |
| `Tls` | `id`, `host`, `version`, `cipher`, `alpn`, `elapsed` | TLS handshake completed |
| `Sent` | `id`, `method`, `path`, `elapsed` | Request dispatched (`elapsed` is zero on H2/H3 before the driver sends) |
| `Head` | `id`, `host`, `status`, `headers`, `elapsed` | Response head arrived (raw `&http::HeaderMap` before decompression/redirects) |
| `Done` | `id`, `elapsed`, `outcome` (`Result<(), &Error>`) | One transport attempt finished |
