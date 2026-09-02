# Requests

`Session::get` and its siblings return a `RequestBuilder`. You chain settings
on the builder and then await it.

## The http types

The public surface speaks the `http` crate, the way reqwest and hyper do.
Leyline re-exports it as `leyline::http`, so you and the client share one
version of `Method`, `Uri`, `StatusCode`, `HeaderName`, and `HeaderValue`.

## Methods

The session has one method per common verb: `get`, `post`, `put`, `patch`,
`delete`, and `head`. For anything else, call `request` with an `http::Method`
and anything that parses as an `http::Uri`.

```rust,no_run
use leyline::http::Method;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::chrome();
let resp = session.request(Method::OPTIONS, "https://example.com/api").await?;
println!("{}", resp.status());
# Ok(())
# }
```

A URL that does not parse as a `Uri` does not panic and does not fail at the
call. The builder records the error, and `send` returns it.

## Headers

Header setters take anything that converts to an `http::HeaderName` and an
`http::HeaderValue`, so `&str` and `String` keep working, and a `HeaderName`
constant costs no parse. An invalid name or value surfaces as an error from
`send`, not at the call site.

`header` replaces every earlier value with the same name. `append_header`
keeps them.

```rust,no_run
use leyline::http::header::ACCEPT_LANGUAGE;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::chrome();
let resp = session
    .get("https://example.com/")
    .header("x-request-id", "1")
    .header(ACCEPT_LANGUAGE, "en-GB,en;q=0.9")
    .append_header("x-tag", "a")
    .append_header("x-tag", "b")
    .await?;
# let _ = resp;
# Ok(())
# }
```

`headers` and `append_headers` take an iterator of pairs and follow the same
replace-or-keep rule. There are named shorthands for the headers you set most:
`accept`, `accept_language`, `user_agent`, `referer`, `origin`,
`content_type`, `bearer_auth`, and `basic_auth`.

### Order

Order matters to a fingerprint, so Leyline preserves it. Your headers merge
into the profile's preset block, and the profile's own order applies on the
wire.

Two escape hatches let you take control:

- `header_order(&["a", "b"])` pins the wire order of the regular headers for
  this request, on every protocol. It wins over the identity's own order.
- `anchored(anchor, name, value)` inserts one header at a named slot, such as
  immediately after `user-agent`.

```rust,no_run
use leyline::profile::HeaderAnchor;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::chrome();
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
let session = leyline::Session::chrome();
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
let session = leyline::Session::chrome();

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

`form_str` sends a form body you encoded yourself. `compress(encoding)`
compresses a buffered body and sets the matching `Content-Encoding` header.

### Multipart

The `multipart` feature, on by default, adds `leyline::multipart::{Form, Part}`
and the `multipart` builder method. `Form::file` streams the file from disk
chunk by chunk, so a large upload is never fully materialized in memory.

```rust,no_run
use leyline::multipart::{Form, Part};

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::chrome();
let form = Form::new()
    .text("name", "ada")
    .part("note", Part::text("hello").mime("text/plain"));
let resp = session.post("https://example.com/upload").multipart(form).await?;
# let _ = resp;
# Ok(())
# }
```

### Streaming bodies

`Body::stream` wraps any `Stream` of `io::Result<Bytes>`.
`Body::stream_with_length` does the same and declares an exact
`Content-Length`. Give the length whenever you know it: a body with no length
hint is sent chunked. See [Streaming](streaming.md).

```rust,no_run
use bytes::Bytes;
use leyline::Body;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::chrome();
let chunks = (0..4).map(|_| Ok::<Bytes, std::io::Error>(Bytes::from_static(b"data")));
let body = Body::stream_with_length(futures_util::stream::iter(chunks), 16);
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
let session = leyline::Session::chrome();
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
let session = leyline::Session::chrome();
let resp = session
    .get("https://example.com/ip")
    .proxy("http://user:pass@proxy.example:8080")
    .timeout(Duration::from_secs(10))
    .await?;
# let _ = resp;
# Ok(())
# }
```

`timeout` replaces the total request timeout alone. `timeouts` replaces
`total`, `read`, and `response_header` together for this one request.
`connect` stays session-wide, because connections are pooled and coalesced
across requests.

```rust,no_run
use leyline::TimeoutConfig;
use std::time::Duration;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::chrome();
let resp = session
    .get("https://example.com/slow")
    .timeouts(
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

## The Request value type

`Request` is an owned request you can build, store, and send later. `new`
takes an `http::Method` and anything that parses as an `http::Uri`. Its
settings mirror the builder's: `header`, `body`, `timeout`, `retry`,
`allow_non_idempotent_retry`, `digest_auth`, and `stream`. `Session::execute`
sends it.

```rust,no_run
use leyline::Request;
use leyline::http::Method;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::chrome();
let req = Request::new(Method::POST, "https://example.com/items")
    .header("content-type", "application/json")
    .body(r#"{"ok":true}"#);
let resp = session.execute(req).await?;
# let _ = resp;
# Ok(())
# }
```

`execute` infers the preset from `content-type` the same way the builder does,
unless the `Request` already carries one. A `Request` is forgiving where the
builder is strict: an unparsable URL becomes the default `Uri` and surfaces
when the session executes it, and an invalid header is dropped.

## Tower service

The `tower` feature, off by default, adds `LeylineService`. It wraps a session
and implements `tower_service::Service` twice:

- `Service<leyline::Request>`, answering with a `leyline::Response`.
- `Service<http::Request<Body>>`, answering with an `http::Response<Body>`, so
  a stack built on `http` types drops in unchanged.

Both use `leyline::Error` as the error type. The `http` adapter carries the
method, the URI, the headers, and the body across; the version and the
extensions are dropped, because the profile picks the protocol. The response
body comes back as a stream.

```rust,no_run
# #[cfg(feature = "tower")]
# async fn run() -> leyline::Result<()> {
use leyline::http::{Method, Request as HttpRequest};
use leyline::{Body, LeylineService, Session};
use tower_service::Service;

let mut svc = LeylineService::new(Session::chrome());
let req = HttpRequest::builder()
    .method(Method::GET)
    .uri("https://example.com/")
    .body(Body::Empty)
    .expect("valid request");
let resp = svc.call(req).await?;
println!("{}", resp.status());
# Ok(())
# }
```

Enable it with `features = ["tower"]`.

## Middleware layers

`SessionBuilder::layer`, also behind the `tower` feature, wraps every request
attempt in a Tower stack. The session hands the layer a `layer::Call` after it
resolved the headers, the body, and the proxy, and before it picks a transport.
The layer answers with a `layer::Reply`.

Each redirect leg and each retry attempt is its own `Call`. Redirects, retries,
cookies, and tracing stay in the session, outside the layer, so a layer sees one
attempt and nothing else. A layer can read the method, the URI, the proxy, and
the stream flag, can edit the request headers, and can return a `Reply` without
calling the inner service. It cannot change the protocol policy: the session
picks HTTP/1.1, HTTP/2, or HTTP/3 after the stack returns to the transport.

`layer::Log` is the built-in example. It writes one `tracing` line per call with
the method, the host, the status, and the elapsed time.

```rust,no_run
# #[cfg(feature = "tower")]
# fn build() -> leyline::Result<leyline::Session> {
use leyline::layer::Log;
use leyline::{Browser, Session};

let session = Session::builder()
    .browser(Browser::Chrome147)
    .layer(Log)
    .build()?;
# Ok(session)
# }
```

Compose several layers with `tower::ServiceBuilder` or `tower_layer::Stack` and
pass the composed layer; a second `layer` call replaces the first stack.
`examples/layer.rs` stacks a header-stamping layer under `Log`.

## Next

Read [Responses](responses.md) to read what comes back.
