# Service integration

This page shows how to run Leyline inside a web service or a long-running
worker: share sessions, relay upstream bodies, answer with the right status,
use the Tower adapter, and shut down cleanly.

## Share one session

Build each session once at startup, keep it in the state your framework
gives to each handler, and clone it per handler. Building resolves the
profile, the TLS context, and the trust store; a clone is an `Arc` clone that
shares the pool, the cookie jar, and the identity, so it keeps the warm
connections.

`Session`, `Response`, and `Error` are `Send + Sync`, and a `RequestBuilder`
and the future from `send` are `Send`. `leyline::http` is the `http` 1.x crate, so
its `StatusCode`, `HeaderMap`, and `Request` pass to and from axum 0.8 and
hyper 1 with no conversion.

A service often needs a plain session for its own APIs and a browser session
for sites that check the fingerprint.

> **Gateway / reverse-proxy defaults (important if migrating from `reqwest`):** Unlike `reqwest`, Leyline has **cookies and redirects ON by default** and leaves `tcp_nodelay` unset. For a multi-tenant backend or reverse proxy sharing one `Session` across users, pass `.cookies(None)`, `.redirect(Redirect::none())`, and `.tcp_nodelay(true)` so users never share a cookie jar or experience 40 ms Nagle stalls.

```rust,no_run
use std::time::Duration;
use leyline::trace::Log;
use leyline::{Browser, Platform, Redirect, Session};

#[derive(Clone)]
struct AppState {
    plain: Session,
    browser: Session,
}

impl AppState {
    fn new() -> leyline::Result<Self> {
        let plain = Session::builder()
            .base_url("https://api.example/v1/")
            .cookies(None)
            .redirect(Redirect::none())
            .tcp_nodelay(true)
            .timeout(Duration::from_secs(30))
            .trace(Log)
            .build()?;
        let browser = Session::builder()
            .browser(Browser::Chrome158).platform(Platform::Windows)
            .trace(Log)
            .build()?;
        Ok(Self { plain, browser })
    }
}

# fn run() -> leyline::Result<()> {
let state = AppState::new()?;
let for_handler = state.clone();
# let _ = for_handler;
# Ok(())
# }
```

See [Sessions](sessions.md) for what each kind sends. A buffered body is
capped at 100 MiB by default; set `max_body_size(bytes)` on the session to
what your service accepts from an upstream. See
[Responses](responses.md#size-cap-and-content-coding).

Each request can override the session timeout and proxy with `.timeout(..)`
and `.proxy(..)`. See
[Retries and timeouts](retries-and-timeouts.md#per-request-timeouts) and
[Proxies](proxies.md#per-request-override).

## Relay an upstream body

Send with `.stream()` so that `send()` returns at the response head, then
pass the body on from `bytes_stream()`: a `Body`, which is a
`Stream<Item = leyline::Result<Bytes>>` that most frameworks accept as a
response body. Forward the headers from `relay_headers`, which drops
the hop-by-hop headers of RFC 9110, section 7.6.1.

| Request | `bytes_stream()` gives | `relay_headers()` | `max_body_size` counts |
| --- | --- | --- | --- |
| default | The decoded body | Drops `content-encoding` and `content-length` when encoded | Decoded bytes |
| `.decode(false)` | Raw wire bytes (recommended for pass-through proxies: avoids CPU decompression and keeps `Content-Length`) | Keeps `content-encoding` and `content-length` | Wire bytes |

`leyline::Body` implements `futures_util::Stream<Item = leyline::Result<Bytes>>`
(not `http_body::Body`), so wrap it with your framework's stream adapter (for
example `axum::body::Body::from_stream(resp.bytes_stream()?)`). To bridge an
incoming framework request body into Leyline, wrap its byte stream with
`leyline::Body::stream(stream, content_length)`.

Once your service sends its status and headers, a body error can only abort
the body, and the client sees a truncated response. To answer 502 for an
upstream body over your limit, check `content_length()` before you send the
head, and set `max_body_size` on the request so that a body with no declared
length still stops at the limit:

```rust,no_run
use leyline::Session;

const RELAY_LIMIT: usize = 8 * 1024 * 1024;

# async fn run(session: &Session) -> leyline::Result<()> {
let resp = session
    .get("https://upstream.example/file")
    .max_body_size(RELAY_LIMIT)
    .stream()
    .send()
    .await?;
if resp.content_length().is_some_and(|len| len > RELAY_LIMIT as u64) {
    println!("answer 502");
    return Ok(());
}
let status = resp.status();
let headers = resp.relay_headers();
let body = resp.bytes_stream()?;
# drop((status, headers, body));
# Ok(())
# }
```

After the head, `read` bounds each chunk and `Timeout::body` bounds
the whole body. When your client goes away, drop the stream: Leyline resets
the upstream stream or closes the HTTP/1.1 connection. See
[Streaming](streaming.md) and [Cancellation](cancellation.md).

### Axum 0.8 streaming handler

Because `leyline::Body` implements `futures_util::Stream` rather than
`http_body::Body`, bridge incoming and outgoing bodies explicitly when wiring
an Axum 0.8 reverse-proxy handler:

```rust,ignore
use axum::body::Body as AxumBody;
use axum::extract::{Request, State};
use axum::response::Response;
use futures_util::TryStreamExt;
use leyline::http::StatusCode;
use leyline::{Body, Session};

async fn proxy_handler(
    State(session): State<Session>,
    req: Request,
) -> Result<Response, StatusCode> {
    let (parts, body) = req.into_parts();
    let len = parts
        .headers
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());
    let stream = body.into_data_stream().map_err(std::io::Error::other);
    let upstream_req = leyline::http::Request::from_parts(parts, Body::stream(stream, len));

    let resp = session
        .request_from(upstream_req)
        .decode(false)
        .stream()
        .send()
        .await
        .map_err(|err| status_for(&err))?;

    let mut out = Response::builder().status(resp.status());
    if let Some(headers) = out.headers_mut() {
        *headers = resp.relay_headers();
    }
    let body = resp.bytes_stream().map_err(|err| status_for(&err))?;
    out.body(AxumBody::from_stream(body))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}
```

## Map errors to HTTP statuses

`Kind::gateway_status()` maps `leyline::Error` to an HTTP status: `None` for
`Status` (pass `err.status()` through), `504` for `Timeout`, `503` for
`Overloaded`, `500` for `Url`/`Config`/`Request`, and `502` for everything
else. Check `err.is_shut_down()` first so shutting-down handlers answer `503`
instead of `500`. If a Tower layer wraps the error in `BoxError`, unwrap it
with `leyline::Error::find(&box_err)`. See
[Map errors to HTTP statuses](errors.md#map-errors-to-http-statuses).

```rust
use leyline::http::StatusCode;

fn status_for(err: &leyline::Error) -> StatusCode {
    if err.is_shut_down() {
        return StatusCode::SERVICE_UNAVAILABLE;
    }
    err.kind()
        .gateway_status()
        .or_else(|| err.status())
        .unwrap_or(StatusCode::BAD_GATEWAY)
}

fn error_body(err: &leyline::Error) -> String {
    format!(r#"{{"error":"{}"}}"#, err.kind().as_str())
}
# let _ = (status_for, error_body);
```

`Kind::as_str()` returns a safe snake_case label with no upstream details, and
`leyline::redact_url` masks passwords and queries before logging.

## Log one line per request

Install `leyline::trace::Log` for one `INFO` event per request on
`leyline::trace` (with `version=HTTP/2.0`, redacted URLs, proxy, and attempts),
or `trace::Metrics` / a custom `Trace` hook for counters. See
[Logging and tracing](logging.md#log-each-request).

## Choose an `http` / Tower integration path

| Need | Use | Response mode | Headers |
| --- | --- | --- | --- |
| Idiomatic Leyline client inside handlers | `session.get(..)` / `post(..)` | Buffered (or `.stream()`) | Use `resp.relay_headers()` before forwarding |
| Send an `http::Request<Body>` and get a `Response` | `session.execute(req)` | Buffered | Full `Response` API |
| Modify/stream an `http::Request<Body>` into `http::Response<Body>` | `session.request_from(req).stream().send().await?.into_http()?` | Streaming | Filter with `resp.relay_headers()` before `into_http()` |
| Plug `Session` directly into Tower layers | `tower` feature (`Service<Request<Body>>`) | Streaming | Leaves hop-by-hop headers intact |

## Use the Tower adapter

Enable `features = ["tower"]` (and add `tower-service = "0.3"` with
`use tower_service::Service;`) to use `Session` directly as a
`tower_service::Service<http::Request<leyline::Body>>` returning
`http::Response<leyline::Body>` and `leyline::Error`.

- **Hop-by-hop headers:** `Session::call` calls `into_http()` directly and leaves hop-by-hop headers intact (there is no `relay_headers()` on `http::Response`). For a reverse proxy or gateway that forwards headers, use `session.request_from(req).stream().send().await?` + `resp.relay_headers()` + `resp.into_http()?` directly or inside your own `Service` wrapper (shown below).
- **Body & extensions:** The body streams decoded and capped at `max_body_size` (`leyline::Body` is a `futures_util::Stream`, not `http_body::Body`; wrap with `axum::body::Body::from_stream` when returning from Axum). Request extensions accept `profile::RequestKind`, `Timeout` (merges over session timeouts), `Retry`, `Redirect`, `Proxy`, and `Protocol` (`http::Request::version` is ignored). The response stores the final `leyline::Url` in extensions and sets `version()`.

```rust,no_run
# #[cfg(feature = "tower")]
# async fn run() -> leyline::Result<()> {
use std::time::Duration;
use futures_util::StreamExt;
use leyline::http::Request;
use leyline::{Body, Session, Timeout, Url};
use tower_service::Service;

let mut svc = Session::new();
let mut req = Request::builder()
    .uri("https://example.com/")
    .body(Body::default())
    .expect("valid request");
req.extensions_mut()
    .insert(Timeout::new().total(Duration::from_secs(10)));

let resp = svc.call(req).await?;
let final_url = resp.extensions().get::<Url>().map(Url::as_str);
let version = resp.version();
println!("{} {:?} {:?}", resp.status(), final_url, version);

let mut body = std::pin::pin!(resp.into_body());
let mut size = 0usize;
while let Some(chunk) = body.next().await {
    size += chunk?.len();
}
println!("{size} bytes");
# Ok(())
# }
```

To relay a request without Tower (needs no `tower` feature), strip hop-by-hop
headers with `relay_headers()` before `into_http()`:

```rust,no_run
# mod example {
use leyline::http::{Request, Response};
use leyline::{Body, Session};

pub async fn relay(
    session: Session,
    req: Request<Body>,
) -> leyline::Result<Response<Body>> {
    let resp = session.request_from(req).decode(false).stream().send().await?;
    let headers = resp.relay_headers();
    let (mut parts, body) = resp.into_http()?.into_parts();
    parts.headers = headers;
    Ok(Response::from_parts(parts, body))
}
# }
```

### Write your own service

Wrap `request_from` + `relay_headers` + `into_http` inside a `tower_service::Service`
when you want both Tower middleware and gateway header filtering:

```rust,no_run
# #[cfg(feature = "tower")]
# mod example {
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use leyline::http::{HeaderValue, Request, Response};
use leyline::{Body, Session};
use tower_service::Service;

#[derive(Clone)]
pub struct Upstream {
    session: Session,
}

impl Upstream {
    pub fn new(session: Session) -> Self {
        Self { session }
    }
}

impl Service<Request<Body>> for Upstream {
    type Response = Response<Body>;
    type Error = leyline::Error;
    type Future = Pin<Box<dyn Future<Output = leyline::Result<Response<Body>>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<leyline::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut request: Request<Body>) -> Self::Future {
        request
            .headers_mut()
            .insert("x-forwarded-by", HeaderValue::from_static("gateway"));
        let send = self.session.request_from(request).decode(false).stream().send();
        Box::pin(async move {
            let resp = send.await?;
            let headers = resp.relay_headers();
            let (mut parts, body) = resp.into_http()?.into_parts();
            parts.headers = headers;
            Ok(Response::from_parts(parts, body))
        })
    }
}
# }
```

### Limit and time out upstream calls

Choose the limiter that matches your boundary:

| Mechanism | Scope | When full |
| --- | --- | --- |
| `SessionBuilder::max_connections_per_host` | TCP / H2 sockets per origin | Queues up to `HostLimits::max_pending` (1024 default), then fails with `Kind::Overloaded` (503) |
| `SessionBuilder::host_limits(HostLimits)` | Per-origin & global in-flight requests | Queues up to `max_pending` / `max_total_pending`, then fails with `Kind::Overloaded` (503) |
| `tokio::sync::Semaphore::try_acquire` | Global or per-route gateway shedding | Returns `503 Service Unavailable` immediately without queuing |

```rust,no_run
# #[cfg(feature = "tower")]
# mod example {
use std::sync::Arc;
use std::time::Duration;

use leyline::http::{Request, StatusCode};
use leyline::{Body, Session, Timeout};
use tokio::sync::Semaphore;
use tower_service::Service;

const MAX_IN_FLIGHT: usize = 32;
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct AppState {
    upstream: Session,
    slots: Arc<Semaphore>,
}

impl AppState {
    pub fn new(session: Session) -> Self {
        Self {
            upstream: session,
            slots: Arc::new(Semaphore::new(MAX_IN_FLIGHT)),
        }
    }
}

pub async fn handler(state: AppState, url: String) -> StatusCode {
    let Ok(mut request) = Request::builder().uri(url).body(Body::default()) else {
        return StatusCode::BAD_REQUEST;
    };
    request
        .extensions_mut()
        .insert(Timeout::new().total(UPSTREAM_TIMEOUT));
    let Ok(_slot) = state.slots.try_acquire() else {
        return StatusCode::SERVICE_UNAVAILABLE;
    };
    let mut upstream = state.upstream.clone();
    match upstream.call(request).await {
        Ok(response) => response.status(),
        Err(err) if err.is_shut_down() => StatusCode::SERVICE_UNAVAILABLE,
        Err(err) => err
            .kind()
            .gateway_status()
            .or_else(|| err.status())
            .unwrap_or(StatusCode::BAD_GATEWAY),
    }
}
# }
```

For streaming handlers, hold an `OwnedSemaphorePermit` (`try_acquire_owned`)
inside the returned stream so the slot stays occupied until the body finishes.
When wrapping `Session` with Tower layers, call `ready().await?` (from
`tower::ServiceExt`) before `call(request)` and unwrap `tower::BoxError` with
`leyline::Error::find`.

## Call a service on localhost

`localhost` usually resolves to `::1` and `127.0.0.1`. Happy Eyeballs tries
IPv6 first and IPv4 after `happy_eyeballs_delay` (250 ms by default) or when the
IPv6 attempt fails. For a service that listens only on IPv4, put
`127.0.0.1` in the URL. See [Network](network.md#happy-eyeballs).

## Shut down

Call `Session::shutdown()`, then drop the session. Requests in flight and
new requests on the session, its clones, and its derived sessions fail with
`Kind::Request`, and a streamed body fails on its next read.
`Error::is_shut_down()` identifies this error so handlers can answer `503`.
Pooled connections close when the last `Session` clone is dropped.

```rust,no_run
# async fn run(session: leyline::Session) {
let worker = session.clone();
let task = tokio::spawn(async move { worker.get("https://api.example/jobs").send().await });
session.shutdown().await;
match task.await {
    Ok(Err(err)) if err.is_shut_down() => println!("stopped"),
    Ok(result) => println!("finished: {}", result.is_ok()),
    Err(join) => println!("task failed: {join}"),
}
drop(session);
# }
```

`Session::shutdown()` cancels at once and returns a future; awaiting it waits until every autosave writer of the session and its derived sessions has made its last save. Stop the server first so that the open requests finish, then stop the sessions:

```rust,no_run
use std::time::Duration;

use leyline::{Browser, Session};

# async fn run() -> leyline::Result<()> {
let session = Session::from(Browser::default());
session.autosave("session.json", Duration::from_secs(2));

let worker = session.clone();
let jobs = tokio::spawn(async move { worker.get("https://api.example/jobs").send().await });

tokio::signal::ctrl_c().await?;
session.shutdown().await;
drop(jobs.await);
# Ok(())
# }
```

See [Accounts](accounts.md#save-the-session-as-it-changes).

## Test the service

`leyline::testing::Server` stands in for the upstream. See
[Testing](testing.md), which also shows it standing in for a forward proxy.
