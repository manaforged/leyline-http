# Responses

A `Response` carries the status, the headers, the body, and the metadata
Leyline collected while sending.

## Status

`status()` returns an `http::StatusCode`. Its predicates answer the usual
question without arithmetic: `is_success`, `is_redirection`,
`is_client_error`, and `is_server_error`. Call `as_u16()` when you need the
number.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session.get("https://example.com/").await?;
if resp.status().is_success() {
    println!("ok: {}", resp.status().as_u16());
}
if resp.status() == leyline::http::StatusCode::NOT_MODIFIED {
    println!("cached");
}
# Ok(())
# }
```

`version()` returns `HttpVersion::Http1_1`, `Http2`, or `Http3`. `as_str()`
turns it into `"HTTP/1.1"`, `"HTTP/2"`, or `"HTTP/3"`.

## Headers

`headers()` returns the `http::HeaderMap`, duplicates included. Use
`get`, `get_all`, and iteration as with any `HeaderMap`. Iteration does not
follow wire order. `get_all` yields the values of one name in the order they
arrived.

`header(name)` returns the first value for a name as a string slice,
case-insensitively. It returns `None` when that first value has a byte outside
visible ASCII, even if the bytes are valid UTF-8, and it does not look at later
values. Read `headers()` for the raw bytes.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session.get("https://example.com/").await?;

for (name, value) in resp.headers() {
    println!("{name}: {}", value.to_str().unwrap_or_default());
}

let ctype = resp.header("content-type");
let links: Vec<&str> = resp
    .headers()
    .get_all(leyline::http::header::LINK)
    .iter()
    .filter_map(|v| v.to_str().ok())
    .collect();
# let _ = (ctype, links);
# Ok(())
# }
```

`content_length()` parses the `Content-Length` header. Reading the body does
not change `headers()` or `content_length()`. When Leyline decodes a buffered
body, it removes `Content-Encoding` and `Content-Length` before it returns the
response. A `.stream()` response keeps the wire values.

`request_headers()` reports the headers the session prepared to send, in send
order, after the preset block, the cookie jar, and your own headers were
merged. It requires `.audit(true)` on the session; without audit it returns an
empty list. The values are prepared before dispatch, not captured from the
transport.

## Cookies

The session jar stores `Set-Cookie` automatically. `cookies()` iterates the
`leyline::cookie::Cookie` records parsed from the `Set-Cookie` headers of the
final response. It lists every header that parses, including a deletion (a
`Max-Age=0` or a past `Expires`), and it skips a header that does not parse. It
does not ask the jar, so it can include a cookie that the jar refuses, and it
does not list cookies from redirect legs. The jar is the store: read it with
`session.cookies().get_cookie(&url, name)`. See [Cookies](cookies.md).

## Redirect chain

`url()` returns the final URL as a `&url::Url`. `redirect_chain()` returns the
URLs visited on the way as a `&[url::Url]`, in order, and is empty when nothing
redirected. The chain holds each URL without its user name and password.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session.get("https://example.com/old").await?;
for url in resp.redirect_chain() {
    println!("via {url}");
}
println!("landed on {}", resp.url());
if resp.url().host_str() != Some("example.com") {
    println!("left the site");
}
# Ok(())
# }
```

## Trailers

`trailers()` iterates trailing headers in wire order, as the same
`HeaderName` and `HeaderValue` pairs. Buffered HTTP/2 and
HTTP/3 responses carry them. A streaming response and HTTP/1.1 yield none.

## Bodies

The body is buffered unless the request called `.stream()`. The reading calls
are async and work in both modes: a buffered body returns at once, and a
streamed body is drained first, then decompressed. Draining honors the `read`
timeout for each chunk.

`CompressionConfig::max_body_size` (100 MiB by default) caps a buffered body:
the bytes Leyline holds in memory for `bytes()`, `text()`, and `json()`, and
the decoded body. It applies on HTTP/1.1, HTTP/2, and HTTP/3. A body over the
cap fails with `Kind::Body`, and the message names `max_body_size`.

A streamed body that you read with `into_stream()` or `copy_to()` has no cap.
You read the chunks, so you control the memory, and Leyline does not decode the
bytes.

| Call | Returns | Notes |
| --- | --- | --- |
| `text().await` | `Result<String>` | Decodes with the `Content-Type` charset, default UTF-8. |
| `text_with_charset(label).await` | `Result<String>` | Uses `label` when the response declares no charset. |
| `bytes().await` | `Result<bytes::Bytes>` | Owned body bytes. |
| `json::<T>().await` | `Result<T>` | Deserializes with serde. |
| `into_stream()` | `Result<BodyStream>` | Takes the body as a stream. |
| `copy_to(writer).await` | `Result<u64>` | Streams into any `AsyncWrite`, such as a file. Consumes the response. |
| `read_until(..).await` | See [Streaming](streaming.md) | Stops early on a predicate or a byte limit. |

Every reading call consumes the response, so
`session.get(url).await?.text().await?` is one expression. Read `status()`,
`headers()`, and other metadata before the body call.

`text()` replaces invalid sequences with U+FFFD, and a leading byte order mark
overrides the declared charset. The charset handling comes from the `charset`
feature, on by default. Without it, `text()` falls back to lossy UTF-8.

On a buffered response, `into_stream` hands back the decoded body as one chunk,
and `copy_to` writes it. On a streamed response they give the bytes as sent,
still compressed. The example below is buffered, so the file holds the decoded
body.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session.get("https://example.com/report.csv").await?;
let mut file = tokio::fs::File::create("report.csv").await?;
let written = resp.copy_to(&mut file).await?;
println!("{written} bytes");
# Ok(())
# }
```

## Turn a status into an error

`error_for_status()` consumes the response and returns an error whose kind is
`Kind::Status` for any status at or above 400. The error carries the code as a
`StatusCode` and the final URL as a `url::Url`. `Display` and `Debug` hide the
password and the query.

`error_for_status_ref()` makes the same check on a borrowed response and
returns `Ok(&Response)`. Use it when you need the headers or the body of an
error response, such as a JSON error from an API or a `cf-ray` header.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
match session.get("https://example.com/api").await?.error_for_status() {
    Ok(resp) => println!("ok {}", resp.status()),
    Err(e) if e.is_status() => println!("status {:?}", e.status()),
    Err(e) => return Err(e),
}
# Ok(())
# }
```

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session.get("https://example.com/api").await?;
if let Err(e) = resp.error_for_status_ref() {
    let ray = resp.header("cf-ray").map(str::to_owned);
    let body = resp.text().await?;
    println!("{e}: {body} {ray:?}");
}
# Ok(())
# }
```

`Error::status()` gives you back an `Option<http::StatusCode>`.

## Timing

`timing()` returns a `ResponseTiming` summed across redirect legs:

- `reused`: every leg reused a pooled connection, so no handshake was paid.
- `connect_ms`: DNS, TCP connect, TLS handshake, and HTTP/2 preface for the
  legs that opened a fresh connection. `None` when every leg was warm.
- `send_ms`: request sent to response, in milliseconds.
- `total_ms`: connect plus send.

An HTTP/3 leg records no timing. It adds nothing to `connect_ms`, `send_ms`, or
`total_ms`, and it counts as not reused, so a response that includes an HTTP/3
leg reports `reused` as `false`.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session.get("https://example.com/").await?;
let t = resp.timing();
println!("reused={} total={}ms", t.reused, t.total_ms);
# Ok(())
# }
```

## TLS facts and the audit block

`tls()` returns an `Option<&TlsInfo>` with the negotiated TLS `version`, the
`cipher`, and the peer certificate in DER form (`peer_cert_der`). `version()`
reports the negotiated protocol, which matches the ALPN value.

`audit()` returns the fingerprints this session presents, but only when the
session was built with `.audit(true)`. Otherwise it returns `None`. The block
is computed once per response and cached.

```rust,no_run
# fn run() -> leyline::Result<()> {
# tokio::runtime::Runtime::new().expect("runtime").block_on(async {
let session = leyline::Session::builder()
    .browser(leyline::Browser::default())
    .audit(true)
    .build()?;
let resp = session.get("https://example.com/").await?;
if let Some(a) = resp.audit() {
    println!("ja3={} ja4={} ja4h={} h2={}", a.ja3, a.ja4, a.ja4h, a.h2_fingerprint);
}
# leyline::Result::Ok(())
# })
# }
```

See [Fingerprints](fingerprints.md) for what each field means and how far to
trust it.

## Print a response

`{:?}` prints the status, version, URL, redirect chain, headers, trailers,
request headers, and timing. It does not print the body. The URL and each entry
in the redirect chain print with the password and the query hidden. The value of
an `Authorization`, `Proxy-Authorization`, `Cookie`, or `Set-Cookie` header
prints as `***` wherever it appears.

## Next

Read [Streaming](streaming.md) for bodies too large to buffer.
