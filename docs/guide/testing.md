# Testing

Test code that uses Leyline against a local server. Leyline needs no mock
layer: point a session at `127.0.0.1` and check what the server received.

## Choose the session for a test

| Session | Use it when |
| --- | --- |
| Plain, `Session::new()` | The test checks your own logic: URLs, methods, bodies, headers, error handling. Its default headers are fixed: `user-agent: leyline/<version>`, `accept: */*`, `accept-encoding`, plus `Host` and `Connection: keep-alive` on HTTP/1.1, and `accept-language` only after `.languages(..)`. |
| Browser, `Session::from(..)` | The test checks what the production session sends. Headers and their order come from the profile data, so an exact header assertion can change with the profile. |

Build the session as in production and change only the trust settings and
the base URL. A browser session sends HTTP/1.1 to an `http://` URL and
HTTP/3 only to an origin that advertised it in `Alt-Svc`, so a local server
sees HTTP/1.1 or HTTP/2.

## Add the test server

The `test-util` feature adds `leyline::testing`. List the crate again under
`[dev-dependencies]` with the feature; Cargo unifies the features, so test
builds get `test-util` and release builds do not:

```toml
[dependencies]
leyline-http = "0.3"

[dev-dependencies]
leyline-http = { version = "0.3", features = ["test-util"] }
```

| Item | Does |
| --- | --- |
| `testing::Server::http(handler)` | Starts an HTTP/1.1 server on `127.0.0.1` on a free port |
| `testing::Server::https(handler)` | The same over TLS with a private CA, `http/1.1` only |
| `testing::Server::http_on(listener, handler)` | Synchronous HTTP/1.1 server on a `std::net::TcpListener` you bound (no async runtime needed) |
| `testing::Server::https_on(listener, handler)` | Synchronous HTTPS/1.1 counterpart on a `std::net::TcpListener` with a private CA |
| `queue([..])` | A handler that answers with each `testing::Response` in turn, then with an empty 503 once they run out |
| `url(path)` | `http://127.0.0.1:<port>/path`, or `https://`. `url("")` is the origin with a trailing `/`, for a `base_url` |
| `addr()` | The bound address |
| `tls()` | A `Tls` that trusts only the server's CA |
| `ca_der()` | The CA certificate as DER, for an HTTPS server |
| `requests().await` | Every request received so far, in order |
| `next_request().await` | Yields each request once in arrival order (returning unread buffered ones first, waiting if none, or `None` after stop); `requests()` still lists it |
| `shutdown().await` | Stops the server; dropping it does too |

`testing::Server` speaks HTTP/1.1 over TCP and does not verify client
certificates. For HTTP/2, mTLS, or Unix sockets, run your own test server,
for example with `axum` or `hyper` and `rcgen`.

A handler is any
`Fn(&testing::Request) -> testing::Response + Send + Sync + 'static`. It runs
on a blocking thread. A slow handler delays its own response and any later
request on the same connection.

| `testing::Request` method | Returns |
| --- | --- |
| `method()`, `target()`, `body()` | The method, the request target, and the body |
| `headers()` | Every header in wire order, with the case the client sent |
| `request_line()` | The first line, such as `GET /items HTTP/1.1` |
| `raw()` | The request head as received: request line, headers, and the empty line, byte for byte |
| `text()` | The body as text, with invalid UTF-8 replaced |
| `head()` | The request line and headers as text, without the body, with invalid UTF-8 replaced |
| `header(name)` | The first value of a header. The name match ignores case |
| `header_values(name)` | Every value of a header, in wire order |

Use `raw()` or `header_values(name).len()` to check what a parsed map hides: header case,
order, and duplicates.

| `testing::Response` | Does |
| --- | --- |
| `new(status)` | A response with this status and an empty body |
| `header(name, value)` | Adds a header |
| `body(bytes)` | Sets the body, sent with `content-length` |
| `close()` | Adds `connection: close`. The server closes the connection after it sends the whole response; it does not cut the body |
| `abort_after(bytes)` | Sends the head and the first `bytes` bytes of the body, then drops the connection. A body no longer than `bytes` is sent whole |
| `delay(duration)` | Waits before the status line and headers |
| `chunks(parts, pause)` | Sends a chunked body with `pause` between parts. It replaces `body`, skips empty parts, and adds `transfer-encoding: chunked` unless you set `content-length` or `transfer-encoding` |

See the [`leyline::testing` reference](https://docs.rs/leyline-http/latest/leyline/testing/index.html)
for every item.

## Assert what was sent

Paste each example into a `#[tokio::test] async fn` that returns
`leyline::Result<()>`. Use `server.url(..)`, which holds `127.0.0.1`. With
`localhost`, the first connect attempt can go to `::1`, where the server does
not listen.

```rust,no_run
# #[cfg(feature = "test-util")]
# async fn run() -> leyline::Result<()> {
use leyline::testing;

let server = testing::Server::http(|request| {
    let id = request.header("x-request-id").unwrap_or("none").to_owned();
    testing::Response::new(201).header("x-echo-id", id)
})
.await?;

let session = leyline::Session::new();
let resp = session
    .post(server.url("/items"))
    .header("x-request-id", "test-1")
    .body("hello")
    .send()
    .await?;
assert_eq!(resp.status(), 201);
assert_eq!(resp.header("x-echo-id"), Some("test-1"));

let sent = server.requests().await;
assert_eq!(sent.len(), 1);
assert_eq!(sent[0].method(), "POST");
assert_eq!(sent[0].target(), "/items");
assert_eq!(sent[0].body(), b"hello");
assert_eq!(sent[0].text(), "hello");
assert!(sent[0].head().starts_with("POST /items"));
# Ok(())
# }
```

The server sees the bytes that reached it, which is the strongest check of
what Leyline sent. To see the headers the session prepared, rather than the
bytes that arrived, build the session with `.audit(true)` and read
`Response::request_headers()`; see [Fingerprints](fingerprints.md).

## Answer with a sequence

`queue` answers each request with the next response. Use it for retries and
error handling:

```rust,no_run
# #[cfg(feature = "test-util")]
# async fn run() -> leyline::Result<()> {
use leyline::testing::{self, queue};

let server = testing::Server::http(queue([
    testing::Response::new(503),
    testing::Response::new(200).body("ok"),
]))
.await?;
let session = leyline::Session::builder()
    .retry(3)
    .build()?;
let resp = session.get(server.url("/flaky")).send().await?;
assert_eq!(resp.status(), 200);
assert_eq!(resp.attempts(), 2);
# Ok(())
# }
```

## Cut a body short

`abort_after(bytes)` drops the connection part-way through the body, which
is how to test the handling of a truncated response. The client sees
`Kind::Body` or `Kind::Io`. `close()` does not do this: it sends the whole
body first.

```rust,no_run
# #[cfg(feature = "test-util")]
# async fn run() -> leyline::Result<()> {
use leyline::{Kind, testing};

let server = testing::Server::http(|_| {
    testing::Response::new(200).body("0123456789").abort_after(4)
})
.await?;
let session = leyline::Session::builder().retry(0).build()?;
let err = session
    .get(server.url("/cut"))
    .text().await
    .expect_err("the body ends after 4 of 10 bytes");
assert!(matches!(err.kind(), Kind::Body | Kind::Io));
# Ok(())
# }
```

## Test blocking code

A `leyline::blocking::Session` reaches a `testing::Server` from a plain
`#[test]` function. Start the server with the session's `block_on`:

```rust,no_run
# #[cfg(feature = "test-util")]
# fn run() -> leyline::Result<()> {
use leyline::testing;

let session = leyline::Session::builder().build_blocking()?;
let server = session.block_on(testing::Server::http(|request| {
    testing::Response::new(200).body(request.body().to_vec())
}))?;
let text = session.block_on(session.post(server.url("/echo")).body("ping").text())?;
assert_eq!(text, "ping");
# Ok(())
# }
```

To start the server without `block_on`, use `http_on` or `https_on` from the
table above. `requests()`, `next_request()`, and `shutdown()` stay `async`,
so call them through `block_on`:

```rust,no_run
# #[cfg(feature = "test-util")]
# fn run() -> leyline::Result<()> {
use std::net::TcpListener;
use leyline::testing;

let listener = TcpListener::bind("127.0.0.1:0")?;
let server = testing::Server::https_on(listener, |_| testing::Response::new(200).body("ok"))?;
let session = leyline::Session::builder().tls(server.tls()).build_blocking()?;
assert_eq!(session.block_on(session.get(server.url("/")).text())?, "ok");
assert_eq!(session.block_on(server.requests()).len(), 1);
# Ok(())
# }
```

## Test timeouts

`delay` holds back the head, so it trips `total` and `response_header`.
`chunks` holds back the body, so it trips `read` on a `.stream()` response.

```rust,no_run
# #[cfg(feature = "test-util")]
# async fn run() -> leyline::Result<()> {
use std::time::Duration;
use leyline::Timeout;
use leyline::testing;

const LIMIT: Duration = Duration::from_millis(100);
const STALL: Duration = Duration::from_millis(500);

let server = testing::Server::http(|request| match request.target() {
    "/slow-head" => testing::Response::new(200).delay(STALL),
    _ => testing::Response::new(200).chunks(["first", "second"], STALL),
})
.await?;
let session = leyline::Session::new();

let err = session
    .get(server.url("/slow-head"))
    .timeout(LIMIT)
    .send().await
    .expect_err("the head arrives after the limit");
assert!(err.kind() == leyline::Kind::Timeout);

let resp = session
    .get(server.url("/slow-body"))
    .stream()
    .timeout(Timeout::new().read(LIMIT))
    .send().await?;
let err = resp.text().await.expect_err("the second chunk arrives after the limit");
assert!(err.kind() == leyline::Kind::Timeout);
# Ok(())
# }
```

## Test over TLS

`testing::Server::https` makes a private CA and a leaf certificate for
`localhost` and `127.0.0.1`. `tls()` trusts that CA only, with the system
and environment roots off. The server negotiates HTTP/1.1, so a browser
session uses HTTP/1.1 on it.

```rust,no_run
# #[cfg(feature = "test-util")]
# async fn run() -> leyline::Result<()> {
use leyline::testing;

let server = testing::Server::https(|_| testing::Response::new(200).body("secure")).await?;
let session = leyline::Session::builder()
    .browser(leyline::Browser::default())
    .tls(server.tls())
    .build()?;
let resp = session.get(server.url("/health")).send().await?;
assert_eq!(resp.text().await?, "secure");
# Ok(())
# }
```

### Use your own TLS server

For a server of your own, for example one that speaks HTTP/2, generate a
test CA and a leaf certificate (the `rcgen` crate does this) and trust only
that CA:

```rust,no_run
use leyline::{Session, Tls};

# async fn run(ca_der: Vec<u8>, port: u16) -> leyline::Result<()> {
let trust = Tls::new()
    .system_roots(false)
    .env_roots(false)
    .ca_der(ca_der);
let session = Session::builder().tls(trust).build()?;
let resp = session
    .get(format!("https://127.0.0.1:{port}/health"))
    .send()
    .await?;
assert_eq!(resp.status(), 200);
# Ok(())
# }
```

- Put `127.0.0.1` as an IP address, or `localhost` as a DNS name, in the
  leaf's subject alternative names. To use a host name and stay on IPv4, map
  it with `SessionBuilder::resolve`; see
  [Network](network.md#map-a-host-to-an-address).
- Give the CA a subject name of its own, for example `CN=Test CA`. When the
  leaf's issuer equals its subject, BoringSSL reads the leaf as self-signed
  and the handshake fails with verify code 18, `self signed certificate`.
  Certificate generators often give both the same default name.

See [TLS trust](tls-trust.md).

## Stand in for a forward proxy

`testing::Server` can act as a forward proxy for `http://` URLs. Set its URL as
the proxy: the session sends the request in absolute form, so
`testing::Request::target` holds the full upstream URL, and the upstream host
needs no DNS entry. An `https://` URL needs a `CONNECT` tunnel, which
`testing::Server` does not open.

```rust,no_run
# #[cfg(feature = "test-util")]
# async fn run() -> leyline::Result<()> {
use leyline::Session;
use leyline::testing;

let proxy = testing::Server::http(|_| testing::Response::new(200)).await?;
let session = Session::builder().proxy(proxy.url("")).build()?;
session.get("http://upstream.test/path").send().await?;
let seen = proxy.requests().await;
assert_eq!(seen[0].target(), "http://upstream.test/path");
# Ok(())
# }
```

## Observe requests with a trace hook

A `Trace` hook sees every request, once, after redirects and retries.
Collect the summaries to check the method, the status, and the attempt
count, for example to prove that a retry ran:

```rust,no_run
use std::sync::{Arc, Mutex};
use leyline::trace::{Event, Trace};

#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<(String, Option<u16>, u32)>>,
}

impl Trace for Recorder {
    fn event(&self, event: &Event<'_>) {
        let Event::Summary { method, status, attempts, .. } = event else {
            return;
        };
        if let Ok(mut seen) = self.seen.lock() {
            seen.push(((*method).to_owned(), status.map(|s| s.as_u16()), *attempts));
        }
    }
}

# async fn run(addr: std::net::SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
let recorder = Arc::new(Recorder::default());
let session = leyline::Session::builder()
    .trace(Arc::clone(&recorder))
    .build()?;
let resp = session.get(format!("http://{addr}/")).send().await?;
assert_eq!(resp.status(), 200);

let seen = recorder.seen.lock().map_err(|_| "poisoned")?;
assert_eq!(seen.len(), 1);
assert_eq!(seen[0].0, "GET");
# Ok(())
# }
```

See [Logging and tracing](logging.md#one-event-per-request).
