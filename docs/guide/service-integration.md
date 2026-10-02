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

`Session`, `Response`, `BodyStream`, and `Error` are `Send + Sync`. `Body`
and `RequestBuilder` are `Send`. `leyline::http` is the `http` 1.x crate, so
its `StatusCode`, `HeaderMap`, and `Request` pass to and from axum 0.8 and
hyper 1 with no conversion.

A service often needs a plain session for its own APIs and a browser session
for sites that check the fingerprint:

```rust,no_run
use std::time::Duration;
use leyline::trace::TracingTrace;
use leyline::{Browser, Platform, Session};

#[derive(Clone)]
struct AppState {
    plain: Session,
    browser: Session,
}

impl AppState {
    fn new() -> leyline::Result<Self> {
        let plain = Session::builder()
            .base_url("https://api.example/v1/")
            .timeout(Duration::from_secs(30))
            .trace(TracingTrace)
            .build()?;
        let browser = Session::builder()
            .browser(Browser::default())
            .platform(Platform::Windows)
            .trace(TracingTrace)
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
capped at 100 MiB by default; set `CompressionConfig::max_body_size(bytes)` to
what your service accepts from an upstream. See
[Responses](responses.md#size-cap-and-content-coding).

Each request can override the session timeout and proxy with `.timeout(..)`
and `.proxy(..)`. See
[Retries and timeouts](retries-and-timeouts.md#per-request-timeouts) and
[Proxies](proxies.md#per-request-override).

## Relay an upstream body

Send with `.stream()` so that `send()` returns at the response head, then
pass the body on as a `BodyStream`, a
`Stream<Item = Result<Bytes, std::io::Error>>` that most frameworks accept
as a response body. Forward the headers from `relay_headers`, which drops
the hop-by-hop headers of RFC 9110, section 7.6.1.

| Body | Headers to forward |
| --- | --- |
| `into_stream()`: the bytes as sent, still encoded, no size cap | `relay_headers(RelayBody::AsReceived)` keeps `content-encoding` and `content-length` |
| `into_decoded_stream(limit)`: decoded, capped at `limit` or `max_body_size` | `relay_headers(RelayBody::Decoded)` drops both when the response was encoded |

Once your service sends its status and headers, a body error can only abort
the body, and the client sees a truncated response. To answer 502 for an
upstream body over your limit, check `content_length()` before you send the
head, and relay with `into_decoded_stream(limit)` so that a body with no
declared length still stops at the limit:

```rust,no_run
use leyline::{RelayBody, Session};

const RELAY_LIMIT: u64 = 8 * 1024 * 1024;

# async fn run(session: &Session) -> leyline::Result<()> {
let resp = session.get("https://upstream.example/file").stream().send().await?;
if resp.content_length().is_some_and(|len| len > RELAY_LIMIT) {
    println!("answer 502");
    return Ok(());
}
let status = resp.status();
let headers = resp.relay_headers(RelayBody::Decoded);
let body = resp.into_decoded_stream(Some(RELAY_LIMIT))?;
# drop((status, headers, body));
# Ok(())
# }
```

After the head, `read` bounds each chunk and `TimeoutConfig::body` bounds
the whole body. When your client goes away, drop the stream: Leyline resets
the upstream stream or closes the HTTP/1.1 connection. See
[Streaming](streaming.md) and [Cancellation](cancellation.md).

## Map errors to HTTP statuses

`ErrorCategory::gateway_status()` gives the status a gateway answers for an
error: `None` for `Status` (pass the upstream status through), 504 for
`Timeout`, 500 for `Url`, `Config`, and `Request`, and 502 for the rest.

```rust
use leyline::http::StatusCode;

fn status_for(err: &leyline::Error) -> StatusCode {
    err.category()
        .gateway_status()
        .or_else(|| err.status())
        .unwrap_or(StatusCode::BAD_GATEWAY)
}

fn error_body(err: &leyline::Error) -> String {
    format!(r#"{{"error":"{}"}}"#, err.category().as_str())
}
# let _ = (status_for, error_body);
```

`ErrorCategory::as_str()` is a stable snake_case label that leaks no
upstream detail. Log the failed URL with `leyline::redact_url`, which masks
the password and the query and drops the fragment. See
[Map errors to HTTP statuses](errors.md#map-errors-to-http-statuses) for
the reasons behind each status.

## Log one line per request

`TracingTrace` logs one INFO event per request on the `leyline::trace`
target, after redirects and retries:

```text
INFO leyline::trace: request id=7 method="GET" url=https://***@example.com/a?*** redirects=1 status=200 version=Http2 attempts=1 elapsed_ms=84 streamed=false outcome="ok"
```

For a `.stream()` request the event fires at the head, so `elapsed_ms`
leaves out the body. To feed your own metrics, implement `Trace::summary`;
wrap the hook in an `Arc` to share it between sessions. See
[Logging and tracing](logging.md#one-event-per-request).

## Use the Tower adapter

The `tower` feature adds `LeylineService`, a
`tower_service::Service<http::Request<leyline::Body>>` whose response is
`http::Response<leyline::Body>` and whose error is `leyline::Error`. It sends
each request through the session, with its cookies, redirects, retries, and
presets, and returns at the response head.

- The body streams decoded and capped at `max_body_size`. When Leyline
  decodes it, the response has no `content-encoding` and no
  `content-length`. Trailers are not exposed.
- Every other upstream header stays, hop-by-hop headers included. Filter
  them with `leyline::relay_headers(&headers, RelayBody::AsReceived)`: the
  headers already describe the decoded body.
- Request extensions set per-request policy: `Preset`, `TimeoutConfig`,
  `RetryPolicy`, `RedirectPolicy`, and `ProxyConfig`. A `ProxyConfig`
  bypasses the session's `ProxyPool`. `Session::execute` reads the same
  extensions and returns a buffered `Response`.
- The response carries the final `leyline::Url`, the `HttpVersion`, and the
  `ResponseTiming` as extensions, and sets `version()`.

```rust,no_run
# #[cfg(feature = "tower")]
# async fn run() -> leyline::Result<()> {
use std::time::Duration;
use futures_util::StreamExt;
use leyline::http::Request;
use leyline::{Body, HttpVersion, LeylineService, Session, TimeoutConfig, Url};
use tower_service::Service;

let mut svc = LeylineService::new(Session::new());
let mut req = Request::builder()
    .uri("https://example.com/")
    .body(Body::default())
    .expect("valid request");
req.extensions_mut()
    .insert(TimeoutConfig::new().total(Duration::from_secs(10)));

let resp = svc.call(req).await?;
let final_url = resp.extensions().get::<Url>().map(Url::as_str);
let version = resp.extensions().get::<HttpVersion>();
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

Enable it with `features = ["tower"]`; the example also needs
`tower-service = "0.3"`. The response body is a `leyline::Body`, a
`Stream<Item = std::io::Result<Bytes>>`, so a framework that takes a byte
stream, such as axum with `Body::from_stream`, forwards it as it is. To
forward the response, keep the status and the body and filter the headers:

```rust,no_run
# #[cfg(feature = "tower")]
# mod example {
use leyline::http::{Request, Response};
use leyline::{Body, LeylineService, RelayBody};
use tower_service::Service;

pub async fn relay(
    mut svc: LeylineService,
    req: Request<Body>,
) -> leyline::Result<Response<Body>> {
    let resp = svc.call(req).await?;
    let (mut parts, body) = resp.into_parts();
    parts.headers = leyline::relay_headers(&parts.headers, RelayBody::AsReceived);
    Ok(Response::from_parts(parts, body))
}
# }
```

### Limit and time out upstream calls

`LeylineService` is `Clone`, and a clone shares the session. Build it once,
store it in the application state, and clone it in each handler. To limit
the upstream calls in flight, share one `tokio::sync::Semaphore` in the same
state. To bound one call, put a `TimeoutConfig` in the request extensions; a
timeout is then a `leyline::Error` like any other, and `category()` maps it
to a status:

```rust,no_run
# #[cfg(feature = "tower")]
# mod example {
use std::sync::Arc;
use std::time::Duration;

use leyline::http::{Request, StatusCode};
use leyline::{Body, LeylineService, Session, TimeoutConfig};
use tokio::sync::Semaphore;
use tower_service::Service;

const MAX_IN_FLIGHT: usize = 32;
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct AppState {
    upstream: LeylineService,
    slots: Arc<Semaphore>,
}

impl AppState {
    pub fn new(session: Session) -> Self {
        Self {
            upstream: LeylineService::new(session),
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
        .insert(TimeoutConfig::new().total(UPSTREAM_TIMEOUT));
    let Ok(_slot) = state.slots.acquire().await else {
        return StatusCode::SERVICE_UNAVAILABLE;
    };
    let mut upstream = state.upstream.clone();
    match upstream.call(request).await {
        Ok(response) => response.status(),
        Err(err) => err
            .category()
            .gateway_status()
            .or_else(|| err.status())
            .unwrap_or(StatusCode::BAD_GATEWAY),
    }
}
# }
```

`LeylineService` is a `tower_service::Service`, so `tower` layers can wrap it
too. A layer returns `tower::BoxError` and can wrap the `leyline::Error` in
its own error; `Error::find` walks the `source()` chain and returns the first
`leyline::Error`. See [Map errors to HTTP statuses](#map-errors-to-http-statuses).

## Call a service on localhost

`localhost` usually resolves to `::1` and `127.0.0.1`. Happy Eyeballs tries
IPv6 first and IPv4 after `resolve_delay` (250 ms by default) or when the
IPv6 attempt fails. For a service that listens only on IPv4, put
`127.0.0.1` in the URL. See [Network](network.md#happy-eyeballs).

## Shut down

Call `Session::shutdown()`, then drop the session. Requests in flight and
new requests on the session, its clones, and its derived sessions fail with
`Kind::Request`, and a streamed body fails on its next read.
`Error::is_shut_down()` identifies this error, so a handler can answer 503.
The session holds nothing to flush, and the pooled connections stay open.

```rust,no_run
# async fn run(session: leyline::Session) {
let worker = session.clone();
let task = tokio::spawn(async move { worker.get("https://api.example/jobs").await });
session.shutdown();
match task.await {
    Ok(Err(err)) if err.is_shut_down() => println!("stopped"),
    Ok(result) => println!("finished: {}", result.is_ok()),
    Err(join) => println!("task failed: {join}"),
}
drop(session);
# }
```

`Session::shutdown()` is a plain call. The autosave handles of a cookie jar
and a device are separate: `JarAutosave::shutdown()` and
`DeviceAutosave::shutdown()` are `async`, save the last changes, and return
the result of that save. Stop the server first so that the open requests
finish, then stop the sessions, then await each autosave handle:

```rust,no_run
use std::time::Duration;

use leyline::{Browser, Session};

# async fn run() -> leyline::Result<()> {
let session = Session::browser(Browser::default());
let cookies = session.cookies().autosave("cookies.json", Duration::from_secs(2));

let worker = session.clone();
let jobs = tokio::spawn(async move { worker.get("https://api.example/jobs").await });

tokio::signal::ctrl_c().await?;
session.shutdown();
drop(jobs.await);
cookies.shutdown().await?;
# Ok(())
# }
```

For a device, `Device::autosave` returns a `DeviceAutosave`; await its
`shutdown()` in the same place. See [Accounts](accounts.md#save-the-device-as-it-changes).

## Test the service

`leyline::testing::TestServer` stands in for the upstream. See
[Testing](testing.md), which also shows it standing in for a forward proxy.
