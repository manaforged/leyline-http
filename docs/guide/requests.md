# Requests

`Session::get` and its siblings return a `RequestBuilder`. You chain settings
on the builder and then await it.

## The http types

The public surface speaks the `http` crate, the way reqwest and hyper do.
Leyline re-exports it as `leyline::http`, so you and the client share one
version of `Method`, `Uri`, `StatusCode`, `HeaderName`, and `HeaderValue`.

## Methods

The session has one method per common verb: `get`, `post`, `put`, `patch`,
`delete`, and `head`. For anything else, call `request` with an `http::Method`.
Every one of them takes `impl IntoUrl`: a `&str`, a `String`, a `&String`, a
`url::Url`, or a `&url::Url`.

```rust,no_run
use leyline::http::Method;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session.request(Method::OPTIONS, "https://example.com/api").await?;
println!("{}", resp.status());
# Ok(())
# }
```

A URL that does not parse does not panic and does not fail at the call. The
builder records the error, and `send` returns it as `Kind::Url` with the
`url::ParseError` as its source.

## Headers

Header setters take anything that converts to an `http::HeaderName` and an
`http::HeaderValue`, so `&str` and `String` keep working, and a `HeaderName`
constant costs no parse. An invalid name or value surfaces as an error from
`send`, not at the call site.

`header` appends. A second call with the same name adds a second value and
keeps the first.

```rust,no_run
use leyline::http::header::ACCEPT_LANGUAGE;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session
    .get("https://example.com/")
    .header("x-request-id", "1")
    .header(ACCEPT_LANGUAGE, "en-GB,en;q=0.9")
    .header("x-tag", "a")
    .header("x-tag", "b")
    .await?;
# let _ = resp;
# Ok(())
# }
```

`headers` takes an iterator of pairs and appends each one the same way. Set
`accept`, `user-agent`, `referer`, and other named headers with `header`.
`bearer_auth` and `basic_auth` build the `Authorization` header for you.

### Order

Order matters to a fingerprint, so Leyline preserves it. Your headers merge
into the profile's preset block, and the profile's own order applies on the
wire. A request header replaces a profile or `SessionBuilder::headers` header
of the same name and takes its slot; repeated `header` calls for that name
send every value there. See the header merge rule in the
[API map](../api.md).

Two methods override the header order:

- `header_order(&["a", "b"])` pins the wire order of the regular headers for
  this request, on every protocol. It wins over the order of the profile's header shape.
- `anchored(anchor, name, value)` inserts one header at a named slot, such as
  immediately after `user-agent`.

```rust,no_run
use leyline::HeaderAnchor;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session
    .get("https://example.com/")
    .anchored(HeaderAnchor::AfterUserAgent, "x-client", "leyline")
    .header_order(&["x-client", "accept"])
    .await?;
# let _ = resp;
# Ok(())
# }
```

The `HeaderAnchor` slots are `AfterCchUa`, `AfterCchUaMobile`,
`AfterCchUaPlatform`, `AfterUserAgent`, `AfterAccept`, `AfterContentType`, and
`BeforeAcceptEncoding`.

To see the headers a request actually sent, read `Response::request_headers`.
See [Responses](responses.md).

## Query parameters

`query` appends pairs to the URL's query string. Call it more than once to add
more.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session
    .get("https://example.com/search")
    .query([("q", "leyline"), ("page", "2")])
    .await?;
# let _ = resp;
# Ok(())
# }
```

## Bodies

`Body` is either empty, a buffered `Bytes` buffer, or a stream. `From` impls
cover `String`, `&'static str`, `Vec<u8>`, `&'static [u8]`, `Bytes`, and `()`,
so `body` takes any of them directly.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();

// Raw bytes.
let a = session
    .post("https://example.com/raw")
    .header("content-type", "application/octet-stream")
    .body(vec![0u8, 1, 2])
    .await?;

// JSON. Sets content-type: application/json.
let b = session
    .post("https://example.com/items")
    .json(&serde_json::json!({ "name": "leyline" }))
    .await?;

// URL-encoded form. Sets content-type: application/x-www-form-urlencoded.
let c = session
    .post("https://example.com/login")
    .form([("user", "ada"), ("pass", "hunter2")])
    .await?;

# let _ = (a, b, c);
# Ok(())
# }
```

To send a form body you encoded yourself, set the
`content-type: application/x-www-form-urlencoded` header and pass the string
to `body`. `compress(encoding)`
compresses a buffered body and sets the matching `Content-Encoding` header.

### Multipart

The `multipart` feature, on by default, adds `leyline::multipart::{Form, Part}`
and the `multipart` builder method. `Form::file` streams the file from disk
chunk by chunk, so a large upload is never fully materialized in memory.

```rust,no_run
use leyline::multipart::{Form, Part};

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let form = Form::new()
    .text("name", "ada")
    .part("note", Part::text("hello").mime("text/plain"));
let resp = session.post("https://example.com/upload").multipart(form).await?;
# let _ = resp;
# Ok(())
# }
```

### Streaming bodies

`Body::stream(s, len)` wraps any `Stream` of `io::Result<Bytes>`. Pass
`Some(n)` to declare an exact `Content-Length`, or `None`. Give the length whenever you know it: a body with no length
hint is sent chunked. See [Streaming](streaming.md). The example needs `bytes`
and `futures-util` in your manifest.

```rust,no_run
use bytes::Bytes;
use leyline::Body;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let chunks = (0..4).map(|_| Ok::<Bytes, std::io::Error>(Bytes::from_static(b"data")));
let body = Body::stream(futures_util::stream::iter(chunks), Some(16));
let resp = session
    .post("https://example.com/upload")
    .header("content-type", "application/octet-stream")
    .body(body)
    .await?;
# let _ = resp;
# Ok(())
# }
```

## Presets and content-type inference

A `Preset` decides the `sec-fetch-*` headers and the header order for a fetch
context: `Navigate`, `Script`, `Xhr`, `Form`, `CrossOrigin`, `SameSite`, and
`FormNavigate`.

When the session impersonates a browser and you set no preset, a POST, PUT, or
PATCH infers one from the `content-type` header: `application/json` gives
`Preset::Xhr`, and `application/x-www-form-urlencoded` gives `Preset::Form`.
Any other content type leaves the preset unset. `json()` and `form()` set that
header, so the common cases need no preset at all.

Set one explicitly when the default guess is wrong.

```rust,no_run
use leyline::Preset;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let js = session
    .get("https://example.com/app.js")
    .preset(Preset::Script)
    .await?;
# let _ = js;
# Ok(())
# }
```

## Per-request proxy and timeouts

`proxy` and `timeout` override the session for one request.

```rust,no_run
use std::time::Duration;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session
    .get("https://example.com/ip")
    .proxy("http://user:pass@proxy.example:8080")
    .timeout(Duration::from_secs(10))
    .await?;
# let _ = resp;
# Ok(())
# }
```

`timeout` takes a `Duration` or a `TimeoutConfig`. A `Duration` sets the total
request timeout alone. A `TimeoutConfig` sets `total`, `read`, and
`response_header` for this one request. A `None` keeps
the session value for that field, so a session `read` or `response_header`
timeout cannot be disabled per request. `connect` stays session-wide,
because connections are pooled and coalesced across requests.

```rust,no_run
use leyline::TimeoutConfig;
use std::time::Duration;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session
    .get("https://example.com/slow")
    .timeout(
        TimeoutConfig::default()
            .total(Duration::from_secs(30))
            .read(Duration::from_secs(5)),
    )
    .await?;
# let _ = resp;
# Ok(())
# }
```

See [Retries and timeouts](retries-and-timeouts.md).

## Send an http::Request

`Session::execute` sends an `http::Request<Body>`. It carries the method, the
URI, the headers, and the body across. It reads three optional values from the
request extensions: a `Preset`, a `TimeoutConfig`, and a `RetryPolicy`.

```rust,no_run
use leyline::Body;
use leyline::http::{Method, Request};

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let req = Request::builder()
    .method(Method::POST)
    .uri("https://example.com/items")
    .header("content-type", "application/json")
    .body(Body::from(r#"{"ok":true}"#))
    .expect("valid request");
let resp = session.execute(req).await?;
# let _ = resp;
# Ok(())
# }
```

`execute` infers the preset from `content-type` the same way the builder does,
unless the extensions already carry one. Digest authentication and response
streaming are not available through `execute`. Use the `RequestBuilder` for
them.

## Tower service

The `tower` feature, off by default, adds `LeylineService`. It wraps a session
and implements `tower_service::Service<http::Request<Body>>`, answering with an
`http::Response<Body>`, so a stack built on `http` types drops in unchanged.
The error type is `leyline::Error`.

The service sends each request through `Session::execute`. The version is
dropped, because the profile picks the protocol. The response body comes back
as a stream. To add middleware, wrap the service with any Tower layer.
Redirects, retries, cookies, and tracing stay in the session, inside the
service.

```rust,no_run
# #[cfg(feature = "tower")]
# async fn run() -> leyline::Result<()> {
use leyline::http::{Method, Request as HttpRequest};
use leyline::{Body, LeylineService, Session};
use tower_service::Service;

let mut svc = LeylineService::new(Session::new());
let req = HttpRequest::builder()
    .method(Method::GET)
    .uri("https://example.com/")
    .body(Body::default())
    .expect("valid request");
let resp = svc.call(req).await?;
println!("{}", resp.status());
# Ok(())
# }
```

Enable it with `features = ["tower"]`. The example also needs
`tower-service = "0.3"` in your manifest; the feature does not re-export it.

## Next

Read [Responses](responses.md) to read what comes back.
