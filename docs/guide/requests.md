# Requests

`Session::get` and its siblings return a `RequestBuilder`. Chain settings on
the builder, then await it. This chapter covers methods, headers, bodies, the
fetch context of a browser request, and per-request overrides. The API uses
the `http` crate's types, re-exported as `leyline::http`.

## Methods and URLs

The session has `get`, `post`, `put`, `patch`, `delete`, and `head`. For any
other method, call `request` with an `http::Method`. Each takes a `&str`, a
`String`, a `&String`, a `url::Url`, or a `&url::Url`.

```rust,no_run
use leyline::http::Method;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session.request(Method::OPTIONS, "https://example.com/api").send().await?;
println!("{}", resp.status());
# Ok(())
# }
```

A relative `&str` or `String` URL resolves against the session base URL; see
[Set a token and a base URL](sessions.md#set-a-token-and-a-base-url). A URL
that does not parse does not panic: `send` returns `Kind::Url` with the
`url::ParseError` as its source.

## Headers

| Setter | Effect |
| --- | --- |
| `header(name, value)` | Appends a value (accepts `&str`, `String`, or `http::HeaderName`/`HeaderValue`) |
| `headers(pairs)` | Appends each `(name, value)` pair from an iterator |
| `bearer_auth(token)` | Sets `Authorization: Bearer <token>` for this request |
| `basic_auth(user, pass)` | Sets `Authorization: Basic <credentials>` for this request |
| `digest_auth(user, pass)` | Answers an HTTP Digest `401` challenge automatically |

Invalid header names or values fail `send()`. On a browser session, setting a
per-request `user-agent` header drops the profile's `sec-ch-ua*` client hints
unless the request sets them too. On HTTP/2 and HTTP/3, a custom `Host` header
is ignored because `:authority` carries the host.

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
    .send().await?;
# let _ = resp;
# Ok(())
# }
```

On a browser session, see
[Override headers safely](fingerprints.md#override-headers-safely) for the
headers you can change without breaking the fingerprint.

### Order

Order matters to a fingerprint, so the profile's order applies on the wire. A
request header replaces every session or profile header of the same name, in
that header's position; repeated `header` calls for the name send every value
there. A name the session does not send goes where a browser places it, such
as `authorization` after `user-agent`, or else at the end.

Two methods override the order:

- `header_order(&["a", "b"])` pins the wire order of the regular headers for
  this request, on every protocol.
- `header_at(position, name, value)` inserts one header at a `HeaderPosition`
  slot: `BeforeSecChUa`, `AfterSecChUa`, `AfterSecChUaMobile`, `AfterSecChUaPlatform`,
  `AfterUserAgent`, `AfterAccept`, `AfterContentType`, `AfterSecFetchDest`, or
  `BeforeAcceptEncoding`.

```rust,no_run
use leyline::profile::HeaderPosition;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session
    .get("https://example.com/")
    .header_at(HeaderPosition::AfterUserAgent, "x-client", "leyline")
    .header_order(&["x-client", "accept"])
    .send().await?;
# let _ = resp;
# Ok(())
# }
```

To see the headers a request was prepared with, build the session with
`.audit(true)` and read `Response::request_headers`.

## Query parameters and bodies

`query` appends pairs to the URL after any existing query; `form` sends pairs
as `application/x-www-form-urlencoded`. Both accept an array (`[("k", "v")]`),
`Vec`, slice, or `HashMap` of string pairs. One request can carry both.

`body` takes a `Body`, or anything that converts to one: `String`,
`&'static str`, `Vec<u8>`, `&'static [u8]`, `Bytes`, or `()`. `json` sets
`content-type: application/json`. `compress_body(encoding)` compresses a buffered
body and sets `Content-Encoding`.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let search = session
    .post("https://example.com/search")
    .query([("page", "2")])
    .form([("q", "leyline"), ("sort", "new")])
    .send().await?;
let item = session
    .post("https://example.com/items")
    .json(&serde_json::json!({ "name": "leyline" }))
    .send().await?;
let raw = session
    .post("https://example.com/raw")
    .header("content-type", "application/octet-stream")
    .body(vec![0u8, 1, 2])
    .send().await?;
# let _ = (search, item, raw);
# Ok(())
# }
```

`Body::file(path)?` streams a regular file from disk and sends its size as
`Content-Length`; it fails with `Kind::Io` if the path cannot be opened or is
not a regular file.

```rust,no_run
use leyline::Body;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session
    .put("https://example.com/upload")
    .body(Body::file("upload.bin")?)
    .send().await?;
# let _ = resp;
# Ok(())
# }
```

`Body::as_bytes()` returns buffered bytes (`None` for a stream),
`Body::content_length()` returns the known byte length for buffered, file, and
sized-stream bodies, and `Body::try_clone()` clones a buffered body. To stream
from any async source, pass `Body::stream`; see
[Streaming](streaming.md#stream-a-request-body).

### Upload multipart forms

The `multipart` feature, on by default, adds `leyline::multipart::{Form, Part}`
and `RequestBuilder::multipart`, which sets `content-type:
multipart/form-data` with the form's boundary. A file part streams from disk
chunk by chunk.

```rust,no_run
use leyline::multipart::{Form, Part};

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let form = Form::new()
    .part("name", Part::text("ada"))
    .part("log", Part::file("server.log")?)
    .part("logo", Part::file("logo.png")?.mime("image/png"))
    .part(
        "photo",
        Part::bytes(std::fs::read("photo.jpg")?)
            .filename("photo.jpg")
            .mime("image/jpeg"),
    );
let resp = session.post("https://api.example/upload").multipart(form).send().await?;
# let _ = resp;
# Ok(())
# }
```

`Part::file(path)` uses the last component of `path` as the filename (or `file`
when missing or non-UTF-8) and sends no part `Content-Type` by default (RFC 7578
`text/plain`); call `.mime(..)` to set one. Unopenable files fail with
`Kind::Io`. See `examples/multipart.rs` for a complete example.

## Override the session for one request

| Call | Effect for this request |
| --- | --- |
| `proxy(url)` | Sends through this proxy |
| `timeout(..)` | Overrides `total`, `read`, `body`, `response_header`, and `error_body` (`connect` stays session-wide) |
| `redirect(policy)` | Overrides the redirect policy; see [Redirects](redirects.md) |
| `retry(policy)` | Overrides the retry policy |
| `protocol(Protocol)` | Forces the HTTP version for this request and its redirects on the session's pool |
| `max_body_size(n_or_none)` | Overrides or removes (`None`) the decoded body cap |
| `decode(bool)` | Turns automatic content decompression on or off |
| `label(text)` | Names the request in the trace summary and `Log` output; see [Logging and tracing](logging.md) |

To use another cookie jar for some requests, derive a session with
`session.to_builder().cookies(jar).build()?`; see
[Derive a session](sessions.md#derive-a-session).

- **`timeout(..)`**: A `Duration` sets `total` (which ends at the response head for `.stream()`); a `Timeout` merges `total`, `read`, `body`, `response_header`, and `error_body` over the session defaults (`None` turns a timeout off). See [Retries and timeouts](retries-and-timeouts.md).
- **`protocol(..)`**: A version the server cannot speak fails the request (for example, `Protocol::Http2` against an HTTP/1.1 server fails with `Kind::Http2`). Requests with a forced version bypass reading/filling the HTTP cache (unsafe writes still invalidate cached entries), and WebSocket handshakes honor it too. Forced HTTP/1.1 requests keep their own pooled connections apart from HTTP/2 connections to the same `https` host.

```rust,no_run
use std::time::Duration;

use leyline::Timeout;
use leyline::cookie::Jar;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let guest = Jar::new();
let resp = session
    .to_builder()
    .cookies(guest.clone())
    .build()?
    .get("https://example.com/slow")
    .proxy("http://user:pass@proxy.example:8080")
    .timeout(
        Timeout::new()
            .total(Duration::from_secs(30))
            .read(Duration::from_secs(5)),
    )
    .label("account-7")
    .send().await?;
println!("{} cookies", guest.len());
let legacy = session
    .get("https://example.com/legacy")
    .protocol(leyline::Protocol::Http1)
    .send().await?;
# drop((resp, legacy));
# Ok(())
# }
```

## Read, change, or copy a request

- `RequestBuilder::build()?` converts a request into an `http::Request<Body>` without sending it, carrying `RequestKind`, `Timeout`, `Retry`, `Redirect`, `Proxy`, `Protocol`, and download options (`sha256`, `progress`, `resume`) as extensions. Download options are only valid with `download_to()` and fail with `Kind::Config` if sent with `send()`, `text()`, `bytes()`, `pages()`, `events()`, or `websocket()`.
- `Session::request_from(request)` turns an `http::Request<Body>` back into a `RequestBuilder`.
- `RequestBuilder::try_clone()` copies a buffered request and all its options (`None` for a stream body or recorded setter error).

```rust,no_run
use leyline::http::HeaderValue;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let mut request = session
    .post("https://example.com/items")
    .query([("page", "2")])
    .json(&serde_json::json!({ "name": "ada" }))
    .build()?;
println!("{} {} {:?}", request.method(), request.uri(), request.body().as_bytes());
request.headers_mut().insert("x-trace", HeaderValue::from_static("t1"));
let resp = session.request_from(request).send().await?;

let first = session.put("https://example.com/doc").body("payload");
if let Some(second) = first.try_clone() {
    let (a, b) = tokio::join!(first.send(), second.send());
    println!("{} {}", a?.status(), b?.status());
}
# drop(resp);
# Ok(())
# }
```

## Send an http::Request

`Session::execute(request)` sends an `http::Request<Body>` with its method,
URI, headers, and body. It is `session.request_from(request).send()`: it
reads an optional `RequestKind`, `Timeout`, `Retry`, `Redirect`, `Proxy`, and
`Protocol` from the request extensions, infers the request kind from
`content-type` as the builder does, and reads the whole response before it
returns. `http::Request::version` is ignored; to force a version, insert a
`Protocol` extension. To stream the response or use digest authentication,
call `request_from` and set it on the `RequestBuilder`.

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

The `tower` feature makes `Session` a `tower_service::Service` over the
same send path. See
[Service integration](service-integration.md#use-the-tower-adapter).

## Request kinds

A `RequestKind` sets the `sec-fetch-*` headers and the header order for a fetch
context: `Native`, `Navigate`, `FrameNavigate`, `Reload`, `Script`, `Image`,
`Xhr`, `Form`, `CrossOrigin`, `SameSite`, and `FormNavigate`.
`RequestKind::Navigate` is the default for a GET from a browser session.

On a browser session with no request kind, a POST, PUT, or PATCH infers one from
`content-type`: `application/json` gives `Xhr`, and
`application/x-www-form-urlencoded` gives `Form`. Other types leave the
kind unset. Set `.kind(..)` when the guess is wrong.

`Form` is a script that posts form data with `fetch()`. `FormNavigate` is a
person who clicks the submit button of an HTML form, as on a login page;
`form()` never infers it.

| RequestKind | `sec-fetch-mode` | `sec-fetch-dest` | `sec-fetch-user` | Other headers |
| --- | --- | --- | --- | --- |
| `Form` | `cors` | `empty` | none | A script `accept`, `priority: u=1, i` |
| `FormNavigate` | `navigate` | `document` | `?1` | An HTML `accept`, `upgrade-insecure-requests: 1`, `cache-control: max-age=0` |

## Send the requests of a page

Pass the page that sends a request to `from_page`. Leyline then sets
`Referer`, `Origin`, and `sec-fetch-site` the way the browser does for the
request's kind. A navigation with a `from_page` page is a link click. A
navigation without one is a typed URL: `sec-fetch-site: none` and no
`Referer`.

```rust,no_run
use leyline::profile::RequestKind;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::from(leyline::Browser::default());
let page = session.get("https://shop.example/cart").send().await?;
let page_url = page.url().clone();
let cart = session
    .get("https://shop.example/api/cart")
    .kind(RequestKind::Xhr)
    .from_page(page_url.clone())
    .send().await?;
let logo = session
    .get("https://cdn.example/logo.png")
    .kind(RequestKind::Image)
    .from_page(page_url)
    .send().await?;
# drop((cart, logo));
# Ok(())
# }
```

`Referer` follows `strict-origin-when-cross-origin`. P is the page and T the
target:

| P and T | `Referer` |
| --- | --- |
| Same origin | The full page URL, without fragment and user info |
| Same site or cross site | The page origin and `/` |
| P is `https`, T is `http` | None |

`Origin` follows the request kind and relationship between P and T:

| RequestKind | No `from_page` | Same origin | Same site or cross site | P is `https`, T is `http` |
| --- | --- | --- | --- | --- |
| `Xhr` GET | None | None | P origin | P origin |
| `Xhr` POST, `Form` | T origin | T origin | P origin | P origin |
| `FormNavigate` | T origin | T origin | P origin | `null` |
| `Image`, `Script` | None | None | None | None |

`sec-fetch-site` is computed from the page and every redirect by the same rules
for Chromium and Gecko profiles. `sec-fetch-user: ?1` goes only with `Navigate`
and `FormNavigate`. An explicit `referer` header wins over the computed value.

## Keep the page with a tab

A `Tab` keeps the current page, as a browser tab does: each request sends
that page as its `from_page`, and each navigation sets the next page.
`Session::tab()` makes one, and clones of a tab share the page.

| Call | Returns | Sends | Changes page |
| --- | --- | --- | --- |
| `open(url)` | `async -> Result<Response>` | A typed URL: no `from_page` | Yes |
| `follow(url)` | `async -> Result<Response>` | A link click from the current page | Yes |
| `submit(url, fields)` | `async -> Result<Response>` | A clicked form (`RequestKind::FormNavigate`) | Yes |
| `submit_form(&form)` | `async -> Result<Response>` | A parsed `html::Form`, with its own method and action | Yes |
| `fetch(method, url)` | `RequestBuilder` | A script request (`RequestKind::Xhr`; chain `.kind(..)` for `Image`/`Script`) | No |

- `open`, `follow`, `submit`, and `submit_form` are `async fn`s returning `Result<Response>` directly (do not chain `.send()`). Only `fetch` returns a `RequestBuilder`.
- Each navigation response (including `4xx`/`5xx`) updates the current page to the final URL after redirects; transport errors leave it unchanged.
- Relative URLs resolve against the current page, or else the session `base_url`.
- `page(impl Into<Option<Url>>)` sets or clears the current page manually (pass a parsed `Url`, not `&str`). `fetch` on a tab with no current page fails with `Kind::Request` without sending anything.

```rust,no_run
use leyline::profile::RequestKind;

# async fn run() -> leyline::Result<()> {
let tab = leyline::Session::from(leyline::Browser::default()).tab();
tab.open("https://shop.example/").await?;
tab.follow("/cart").await?;
let cart = tab.fetch(leyline::http::Method::GET, "/api/cart").send().await?;
let logo = tab.fetch(leyline::http::Method::GET, "https://cdn.example/logo.png").kind(RequestKind::Image).send().await?;
tab.submit("/checkout", [("step", "address")]).await?;
println!("{:?}", tab.current());
# drop((cart, logo));
# Ok(())
# }
```

[Accounts](accounts.md) uses a tab with a saved session and submits the forms
of a page.
