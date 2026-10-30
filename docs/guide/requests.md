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

Header setters take anything that converts to an `http::HeaderName` and an
`http::HeaderValue`, such as `&str`, `String`, or a `HeaderName` constant. An
invalid name or value is an error from `send`. `header` appends: a second
call with the same name adds a second value. `headers` takes an iterator of
pairs. `bearer_auth` and `basic_auth` build the `Authorization` header. `digest_auth(user, password)` answers an HTTP Digest challenge: the first request gets the `401`, and the session sends it again with the computed `Authorization` header.
On HTTP/2 and HTTP/3 the session does not send a `Host` header you set,
because the request authority carries the host.

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
  slot: `BeforeCchUa`, `AfterCchUa`, `AfterCchUaMobile`, `AfterCchUaPlatform`,
  `AfterUserAgent`, `AfterAccept`, `AfterContentType`, `AfterFetchDest`, or
  `BeforeAcceptEncoding`.

```rust,no_run
use leyline::HeaderPosition;

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

`query` appends pairs to the URL after any query it already has. `form` sends
pairs as `application/x-www-form-urlencoded`. Both take any iterator of
`(K, V)` tuples, or references to them, where `K` and `V` are `AsRef<str>`.
One request can carry both.

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

To stream a request body, pass `Body::stream`; see
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

`Part::file(path)` uses the last component of `path` as the filename, or
`file` when it has none or is not UTF-8. The part has no `Content-Type`, so
the server reads it as `text/plain` (RFC 7578); use
`Part::file(path)?.mime(..)` to send a type. It returns `std::io::Result`,
which `?` converts into `leyline::Error`.
[`examples/multipart.rs`](../../crates/leyline/examples/multipart.rs) is a
complete program.

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
use leyline::RequestKind;

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

| RequestKind | No `from_page` | Same origin | Same site or cross site | P is `https`, T is `http` |
| --- | --- | --- | --- | --- |
| `Xhr` GET | None | None | P origin | P origin |
| `Xhr` POST, `Form` | T origin | T origin | P origin | P origin |
| `FormNavigate` | T origin | T origin | P origin | `null` |
| `Image`, `Script` | None | None | None | None |

The second table gives the `Origin` header. `sec-fetch-site` is computed from
the page and every redirect, by the same rules for Chromium and Gecko
profiles. `sec-fetch-user: ?1` goes only with `Navigate` and `FormNavigate`.
A `referer` header you set wins over the computed value.

## Keep the page with a tab

A `Tab` keeps the current page, as a browser tab does: each request sends
that page as its `from_page`, and each navigation sets the next page.
`Session::tab()` makes one, and clones of a tab share the page.

| Call | Sends | Changes the page |
| --- | --- | --- |
| `open(url)` | A typed URL: no `from_page` | Yes |
| `follow(url)` | A link click from the current page | Yes |
| `submit(url, fields)` | A clicked form, `RequestKind::FormNavigate` | Yes |
| `submit_form(&form)` | A parsed `html::Form`, with its own method and action | Yes |
| `fetch(method, url)` | A script request, `RequestKind::Xhr`; `.kind(..)` makes it an image or script | No |

The page changes after each navigation response, 4xx and 5xx included, to
the final URL after redirects. A transport error leaves it unchanged.
Relative URLs resolve against the current page, or else the base URL. A
`.stream()` navigation changes the page when the response head arrives.
`set_current(page)` sets the page by hand. A `fetch` on
a tab with no page fails with `Kind::Request` and sends nothing: open a page
first.

```rust,no_run
use leyline::RequestKind;

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

## Override the session for one request

| Call | Effect for this request |
| --- | --- |
| `proxy(url)` | Sends through this proxy |
| `timeout(..)` | Overrides the session timeouts |
| `redirect(policy)` | Overrides the redirect policy; see [Redirects](redirects.md) |
| `retry(policy)` | Overrides the retry policy |
| `label(text)` | Names the request in the trace summary and the `Log` line; see [Logging and tracing](logging.md) |

To use another cookie jar for some requests, derive a session with
`session.to_builder().cookies(jar).build()?`; see
[Derive a session](sessions.md#derive-a-session).

`timeout` takes a `Duration` or a `Timeout`. A `Duration` sets `total`,
which bounds the request up to the head of a streamed response; set
`Timeout::body` to bound a streamed body or a download. A
`Timeout` sets `total`, `read`, `body`, and `response_header`; an unset
field keeps the session value, and `None` turns that timeout off. `connect`
is session-wide, because connections are pooled. See
[Retries and timeouts](retries-and-timeouts.md).

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
println!("{} cookies", guest.all().len());
# drop(resp);
# Ok(())
# }
```

## Send an http::Request

`Session::execute` sends an `http::Request<Body>` with its method, URI,
headers, and body. It reads an optional `RequestKind`, `Timeout`,
`Retry`, `Redirect`, and `Proxy` from the request
extensions, infers the request kind from `content-type` as the builder does, and
reads the whole response before it returns. Digest authentication and
response streaming need the `RequestBuilder`.

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
