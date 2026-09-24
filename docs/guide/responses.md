# Responses

A `Response` carries the status, the headers in wire order, the body, and the
metadata Leyline collected while sending.

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

`headers()` iterates `(&http::HeaderName, &http::HeaderValue)` pairs in wire
order, duplicates included.

`header(name)` returns the first value for a name as a string slice,
case-insensitively. It skips a value that is not valid UTF-8, because it hands
you a `&str`; read `headers()` for the raw bytes. To read every value for one
name, filter `headers()`.

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
    .filter(|(k, _)| k.as_str().eq_ignore_ascii_case("link"))
    .filter_map(|(_, v)| v.to_str().ok())
    .collect();
# let _ = (ctype, links);
# Ok(())
# }
```

`content_length()` parses the `Content-Length` header. `request_headers()` reports the headers the session prepared to
send, in send order, after the preset block, the cookie jar, and your own
headers were merged. It requires `.audit(true)` on the session; without audit
it returns an empty list. The values are prepared before dispatch, not
captured from the transport.

## Cookies

The session jar stores `Set-Cookie` automatically. `cookies()` iterates the
`leyline::cookie::Cookie` records parsed from this response's `Set-Cookie`
headers. It does not list cookies from redirect legs or cookies the jar
rejected. The jar is the store: read it with
`session.cookies().get_cookie(url, name)`. See [Cookies](cookies.md).

## Redirect chain

`url()` is the final URL. `redirect_chain()` lists the URLs visited on the way,
in order, and is empty when nothing redirected.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session.get("https://example.com/old").await?;
for hop in resp.redirect_chain() {
    println!("via {hop}");
}
println!("landed on {}", resp.url());
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
streamed body is drained first, then decompressed and kept for later calls.
Draining honors the session `read_timeout` per chunk and the same 100 MiB cap
that buffered mode applies.

| Call | Returns | Notes |
| --- | --- | --- |
| `text().await` | `Result<String>` | Decodes with the `Content-Type` charset, default UTF-8. |
| `text_with_charset(label).await` | `Result<String>` | Uses `label` when the response declares no charset. |
| `bytes().await` | `Result<&[u8]>` | Borrowed raw bytes. Call `.to_vec()` to own them. |
| `json::<T>().await` | `Result<T>` | Deserializes with serde. |
| `into_stream()` | `Result<BodyStream>` | Takes the body as a stream. |
| `copy_to(writer).await` | `Result<u64>` | Streams into any `AsyncWrite`, such as a file. Consumes the response. |
| `read_until(..).await` | See [Streaming](streaming.md) | Stops early on a predicate or a byte limit. |

`text`, `text_with_charset`, `bytes`, and `json` take the response by `&mut`,
so bind it with `let mut resp`.

On a buffered response, `into_stream` and `copy_to` give the decoded body. On a
streamed response they give the bytes as sent, still compressed. The example
below is buffered, so the file holds the decoded body.

`text()` replaces invalid sequences with U+FFFD, and a leading byte order mark
overrides the declared charset. The charset handling comes from the `charset`
feature, on by default. Without it, `text()` falls back to lossy UTF-8.

`into_stream()` works in both modes: a
buffered body is handed back as a one-chunk stream. `into_stream()` consumes
the response, so nothing is left to read after it.

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
`StatusCode`, the URL with its password redacted, and the first 16 KiB of the
body, so a 403 explains itself without a second request. The call does not await, so it
attaches the body only when the body is already buffered; a streamed body gives an error with no body.

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

`Error::status()` gives you back an `Option<http::StatusCode>`.

## Timing

`timing()` returns a `ResponseTiming` summed across redirect legs:

- `reused`: every leg reused a pooled connection, so no handshake was paid.
- `connect_ms`: DNS, TCP connect, TLS handshake, and HTTP/2 preface for the
  legs that opened a fresh connection. `None` when every leg was warm.
- `send_ms`: request sent to response, in milliseconds.
- `total_ms`: connect plus send.

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

## Next

Read [Streaming](streaming.md) for bodies too large to buffer.
