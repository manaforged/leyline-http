# Testing

Test code that uses Leyline against a local server. Leyline needs no mock
layer: point a session at `127.0.0.1` and check what the server received.

## Choose the session for a test

| Session | Use it when |
| --- | --- |
| Plain, `Session::new()` | The test checks your own logic: URLs, methods, bodies, headers, error handling. Its default headers are fixed: `Host`, `user-agent: leyline/<version>`, `accept: */*`, `accept-encoding`, and `Connection` on HTTP/1.1, plus `accept-language` only after `.languages(..)`. |
| Browser, `Session::browser(..)` | The test checks what the production session sends. Headers and their order come from the profile data, so an exact header assertion can change with the profile. |

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
leyline-http = "0.1"

[dev-dependencies]
leyline-http = { version = "0.1", features = ["test-util"] }
```

| Item | Does |
| --- | --- |
| `TestServer::http(handler)` | Starts an HTTP/1.1 server on `127.0.0.1` on a free port |
| `TestServer::https(handler)` | The same over TLS with a private CA, `http/1.1` only |
| `TestServer::http_on(listener, handler)` | An HTTP/1.1 server on a `std::net::TcpListener` you bound, for a fixed port or address. Call it inside a Tokio runtime |
| `queue([..])` | A handler that answers with each `TestResponse` in turn, then 503 |
| `url(path)` | `http://127.0.0.1:<port>/path`, or `https://`. `url("")` is the origin with a trailing `/`, for a `base_url` |
| `addr()` | The bound address |
| `trust()` | A `TlsTrustConfig` that trusts only the server's CA |
| `ca_der()` | The CA certificate as DER, for an HTTPS server |
| `requests().await` | Every request so far, in order |
| `next_request().await` | The next request, when it arrives |
| `shutdown().await` | Stops the server; dropping it does too |

A handler is any `Fn(&RecordedRequest) -> TestResponse + Send + Sync +
'static`. It runs on a blocking thread, so a handler that blocks delays only
its own response.

| `RecordedRequest` | Holds |
| --- | --- |
| `method`, `target`, `body` | The method, the request target, and the body |
| `headers` | Every header in wire order, with the case the client sent |
| `request_line` | The first line, such as `GET /items HTTP/1.1` |
| `raw` | The request head as received: request line, headers, and the empty line, byte for byte |
| `text()` | `raw` as text, with invalid UTF-8 replaced |
| `header(name)` | The first value of a header. The name match ignores case |
| `header_values(name)` | Every value of a header, in wire order |
| `header_count(name)` | How many times a header was sent |

Use `raw` or `header_count` to check what a parsed map hides: header case,
order, and duplicates.

| `TestResponse` | Does |
| --- | --- |
| `new(status)` | A response with this status and an empty body |
| `header(name, value)` | Adds a header |
| `body(bytes)` | Sets the body, sent with `content-length` |
| `close()` | Adds `connection: close`. The server closes the connection after the response |
| `delay(duration)` | Waits before the status line and headers |
| `chunks(parts, pause)` | Sends a chunked body with `pause` between parts. It replaces `body`, skips empty parts, and adds `transfer-encoding: chunked` unless you set `content-length` or `transfer-encoding` |

See the [`leyline::testing` reference](../reference/leyline-http/leyline-testing.md)
for every item.

## Assert what was sent

Put the body of `run` in a `#[tokio::test]` function. Use `server.url(..)`,
which holds `127.0.0.1`: with `localhost`, Leyline tries IPv6 first, where
the server does not listen.

```rust,no_run
# #[cfg(feature = "test-util")]
# async fn run() -> leyline::Result<()> {
use leyline::testing::{TestResponse, TestServer};

let server = TestServer::http(|request| {
    let id = request.header("x-request-id").unwrap_or("none").to_owned();
    TestResponse::new(201).header("x-echo-id", id)
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
assert_eq!(sent[0].method, "POST");
assert_eq!(sent[0].target, "/items");
assert_eq!(sent[0].body, b"hello");
# Ok(())
# }
```

The server sees the bytes that reached it, which is the strongest check of
what Leyline sent. To see what a session prepared without a server, build it
with `.audit(true)` and read `Response::request_headers()`; see
[Fingerprints](fingerprints.md).

## Answer with a sequence

`queue` answers each request with the next response. Use it for retries and
error handling:

```rust,no_run
# #[cfg(feature = "test-util")]
# async fn run() -> leyline::Result<()> {
use leyline::RetryPolicy;
use leyline::testing::{TestResponse, TestServer, queue};

let server = TestServer::http(queue([
    TestResponse::new(503),
    TestResponse::new(200).body("ok"),
]))
.await?;
let session = leyline::Session::builder()
    .retry(RetryPolicy::transient())
    .build()?;
let resp = session.get(server.url("/flaky")).send().await?;
assert_eq!(resp.status(), 200);
assert_eq!(resp.attempts(), 2);
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
use leyline::TimeoutConfig;
use leyline::testing::{TestResponse, TestServer};

const LIMIT: Duration = Duration::from_millis(100);
const STALL: Duration = Duration::from_millis(500);

let server = TestServer::http(|request| match request.target.as_str() {
    "/slow-head" => TestResponse::new(200).delay(STALL),
    _ => TestResponse::new(200).chunks(["first", "second"], STALL),
})
.await?;
let session = leyline::Session::new();

let err = session
    .get(server.url("/slow-head"))
    .timeout(LIMIT)
    .await
    .expect_err("the head arrives after the limit");
assert!(err.is_timeout());

let resp = session
    .get(server.url("/slow-body"))
    .stream()
    .timeout(TimeoutConfig::new().read(LIMIT))
    .await?;
let err = resp.text().await.expect_err("the second chunk arrives after the limit");
assert!(err.is_timeout());
# Ok(())
# }
```

## Test over TLS

`TestServer::https` makes a private CA and a leaf certificate for
`localhost` and `127.0.0.1`. `trust()` trusts that CA only, with the system
and environment roots off. The server negotiates HTTP/1.1, so a browser
session uses HTTP/1.1 on it.

```rust,no_run
# #[cfg(feature = "test-util")]
# async fn run() -> leyline::Result<()> {
use leyline::testing::{TestResponse, TestServer};

let server = TestServer::https(|_| TestResponse::new(200).body("secure")).await?;
let session = leyline::Session::builder()
    .browser(leyline::Browser::default())
    .tls_trust(server.trust())
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
use leyline::{Session, TlsTrustConfig};

# async fn run(ca_der: Vec<u8>, port: u16) -> leyline::Result<()> {
let trust = TlsTrustConfig::new()
    .system_roots(false)
    .env_roots(false)
    .add_ca_der(ca_der);
let session = Session::builder().tls_trust(trust).build()?;
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
  it with `DnsConfig::resolve_host`; see
  [Network](network.md#map-a-host-to-an-address).
- Give the CA a subject name of its own, for example `CN=Test CA`. When the
  leaf's issuer equals its subject, BoringSSL reads the leaf as self-signed
  and the handshake fails with verify code 18, `self signed certificate`.
  Certificate generators often give both the same default name.

See [TLS trust](tls-trust.md).

## Stand in for a forward proxy

`TestServer` can act as a forward proxy for `http://` URLs. Set its URL as
the proxy: the session sends the request in absolute form, so
`RecordedRequest::target` holds the full upstream URL, and the upstream host
needs no DNS entry. An `https://` URL needs a `CONNECT` tunnel, which
`TestServer` does not open.

```rust,no_run
# #[cfg(feature = "test-util")]
# async fn run() -> leyline::Result<()> {
use leyline::Session;
use leyline::testing::{TestResponse, TestServer};

let proxy = TestServer::http(|_| TestResponse::new(200)).await?;
let session = Session::builder().proxy(proxy.url("")).build()?;
session.get("http://upstream.test/path").await?;
let seen = proxy.requests().await;
assert_eq!(seen[0].target, "http://upstream.test/path");
# Ok(())
# }
```

## Observe requests with a trace hook

A `Trace` hook sees every request, once, after redirects and retries.
Collect the summaries to check the method, the status, and the attempt
count, for example to prove that a retry ran:

```rust,no_run
use std::sync::{Arc, Mutex};
use leyline::trace::{Summary, Trace};

#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<(String, Option<u16>, u32)>>,
}

impl Trace for Recorder {
    fn summary(&self, ev: &Summary<'_>) {
        let status = ev.status.map(|s| s.as_u16());
        if let Ok(mut seen) = self.seen.lock() {
            seen.push((ev.method.to_owned(), status, ev.attempts));
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

## Next

Read [Fingerprints](fingerprints.md) to see what a browser session sends on
each layer.
