# Responses

A `Response` carries the status, the headers, the body, and the metadata
Leyline collected while sending: the final URL, the redirect chain, the proxy,
the timing, and the TLS facts. Read the metadata first, because every body
call consumes the response.

## Status and headers

`status()` returns an `http::StatusCode`, with `is_success`,
`is_redirection`, `is_client_error`, and `is_server_error`. `version()`
returns `HttpVersion::Http1_1`, `Http2`, or `Http3`, and `as_str()` gives
`"HTTP/1.1"`, `"HTTP/2"`, or `"HTTP/3"`.

`headers()` returns the `http::HeaderMap`, duplicates included. Iteration
does not follow wire order; `get_all` yields the values of one name in the
order they arrived. `header(name)` returns the first value as `&str`, or
`None` when it is missing or holds a byte outside visible ASCII. Read
`headers()` for the raw bytes.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session.get("https://example.com/").await?;
if resp.status().is_success() {
    println!("{} {}", resp.status().as_u16(), resp.version().as_str());
}
let ctype = resp.header("content-type");
let cookies: Vec<&[u8]> = resp
    .headers()
    .get_all("set-cookie")
    .iter()
    .map(|v| v.as_bytes())
    .collect();
# let _ = (ctype, cookies);
# Ok(())
# }
```

`content_length()` parses `Content-Length`. When Leyline decodes a buffered
body, it removes `Content-Encoding` and `Content-Length` from the headers. A
`.stream()` response keeps the wire values.

`request_headers()` lists the headers the session prepared to send, in send
order, after the preset, the cookie jar, and your headers were merged. It
needs `.audit(true)` on the session and is empty otherwise. The values are
prepared before dispatch, not captured from the transport.

`trailers()` iterates trailing headers in wire order. Buffered HTTP/2 and
HTTP/3 responses carry them; a streamed response and HTTP/1.1 yield none.

### Relay headers

`relay_headers(body)` returns the headers without the hop-by-hop ones:
`connection`, each header that `connection` names, `keep-alive`,
`proxy-connection`, `proxy-authenticate`, `proxy-authorization`, `te`, `trailer`, `transfer-encoding`, and `upgrade` (RFC
9110, section 7.6.1). `RelayBody::AsReceived` matches a body from
`into_stream()`. `RelayBody::Decoded` matches a body from `bytes()`, `text()`,
or `into_decoded_stream()`, and also drops `content-encoding` and
`content-length` when Leyline decodes the body. When the session does not
decode the coding, for example because `CompressionConfig` turns it off, the
body arrives as sent and `content-encoding` stays. `proxy-authenticate` and
`proxy-authorization` are dropped as hop-by-hop headers. See
[Service integration](service-integration.md#relay-an-upstream-body).

## Bodies

Without `.stream()`, Leyline reads, decodes, and caps the whole body before
`send()` returns. With `.stream()`, `send()` returns at the headers. The
reading calls work in both modes: a buffered body returns at once, and a
streamed one is drained, with the `read` timeout on each chunk, then decoded.

| Call | Returns | Notes |
| --- | --- | --- |
| `text().await` | `Result<String>` | Decodes with the `Content-Type` charset, default UTF-8 |
| `text_with_charset(label).await` | `Result<String>` | Uses `label` when the response declares no charset |
| `bytes().await` | `Result<bytes::Bytes>` | The decoded body |
| `json::<T>().await` | `Result<T>` | Deserializes with serde. A failure is `Kind::Json`, in `ErrorCategory::Decode` |
| `into_decoded_stream(limit)` | `Result<BodyStream>` | The decoded body as a stream, capped at `limit` |
| `copy_decoded_to(writer, limit).await` | `Result<u64>` | Writes the decoded body, capped at `limit` |
| `download_to(path, limit).await` | `Result<u64>` | Writes the decoded body to a file in one atomic step |
| `into_stream()`, `copy_to(writer).await` | `Result<BodyStream>`, `Result<u64>` | The bytes as sent on a `.stream()` response; the decoded body on a buffered one |
| `read_until(limit, done).await` | `Result<Vec<u8>>` | Stops early on a predicate or a byte limit |

[Streaming](streaming.md) covers the streaming calls and downloads.

`text()` replaces invalid sequences with U+FFFD, and a leading byte order mark
overrides the declared charset. Without the `charset` feature, on by default,
`text()` uses lossy UTF-8.

### Size cap and content coding

`CompressionConfig::max_body_size` (100 MiB by default) caps a buffered body
and the decoded body, on every protocol. A body over the cap fails with
`Kind::Body`, and the message names `max_body_size`. Set the cap on the
session with `SessionBuilder::compression`:

```rust
use leyline::{CompressionConfig, Session};

# fn run() -> leyline::Result<()> {
let session = Session::builder()
    .compression(CompressionConfig::new().max_body_size(8 * 1024 * 1024))
    .build()?;
# let _ = session;
# Ok(())
# }
```

`CompressionConfig::new()` turns on gzip, deflate, br, and zstd, and a
session decodes and advertises those that are also compiled in.
`CompressionConfig::none()` turns them all off. Repeated `Content-Encoding`
fields combine into one list. More than four codings fail with
`Kind::Decode`. When a listed coding is turned off, the body comes back as
received.

## Final URL and redirects

`url()` returns the final URL after redirects. `redirect_chain()` returns
each URL that answered with a redirect Leyline followed, in order, without
the final URL and without user name or password. It is empty when nothing
redirected. `attempts()` returns the number of transport attempts, 1 when no
retry ran.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session.get("https://example.com/old").await?;
for url in resp.redirect_chain() {
    println!("via {url}");
}
println!("landed on {} after {} attempts", resp.url(), resp.attempts());
# Ok(())
# }
```

See [Redirects](redirects.md).

## Follow `Link` pagination

`link(rel)` returns the URL of the first `Link` entry with that relation.
`links()` returns every entry as a `Link` with `url`, `rel`, and `params`.
Parsing follows RFC 8288: several headers, comma-separated entries, quoted
parameters, and several relations in one `rel`. Relative URLs resolve
against `url()`, and the `rel` match ignores case.

`RequestBuilder::pages()` turns a request into a `Pages` sequence that
follows `Link: rel="next"`. Await `pages.next()`, or use it as a
`futures_util::Stream`. Each follow-up is a `GET` with the first request's
headers, timeouts, retry policy, and `error_for_status()` setting. The
sequence ends after a page with no `next` link, after an error, or at a
`next` URL it already fetched. The first request URL and the final URL of
each page, after redirects, count as fetched.

```rust,no_run
use serde::Deserialize;

#[derive(Deserialize)]
struct Item {
    id: u64,
    name: String,
}

# async fn run() -> leyline::Result<()> {
let api = leyline::Session::builder().bearer_auth("my-token").build()?;
let mut pages = api
    .get("https://api.example/items")
    .error_for_status()
    .pages();
while let Some(page) = pages.next().await {
    for item in page?.json::<Vec<Item>>().await? {
        println!("{} {}", item.id, item.name);
    }
}
# Ok(())
# }
```

## Cookies

The session jar stores `Set-Cookie` automatically. `cookies()` parses the
`Set-Cookie` headers of the final response only. It lists every header that
parses, deletions included, and does not ask the jar, so it can list a cookie
the jar refused. Read the jar with `session.cookies().get_cookie(&url, name)`.
See [Cookies](cookies.md).

## Turn a status into an error

A 4xx or 5xx response is `Ok`. `error_for_status()` consumes the response and
returns a `Kind::Status` error for a status of 400 or more, with the status,
the headers, and the final URL. `error_for_status_ref()` makes the same check
on a borrowed response, so you can still read the body of an error.

`RequestBuilder::error_for_status()` checks the final response after the
retry policy is done, and keeps the start of the body in the error:

| Limit | Default | Set with |
| --- | --- | --- |
| Body bytes kept, after decoding | 64 KiB | `CompressionConfig::max_error_body` |
| Time to read that body | 10 s, and never past the `total` timeout | `TimeoutConfig::error_body` |

Decompression stops at the byte limit, so a small compressed body cannot
expand past it. When the read fails or takes too long, the error has no body
but keeps the status and headers. `download` uses the same limits for a
status error.

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

[Errors](errors.md#status-codes-are-not-errors) covers the status error and
its accessors.

## Proxy, timing, and TLS

`proxy()` returns the proxy that carried the response, with the password, or
a user name without a password, replaced by `***`. It is `None` for a direct
connection. Under a `ProxyPool` it names the proxy of the attempt that
produced the response.

### Timing

`timing()` returns a `ResponseTiming` summed across redirect legs, on every
protocol. It does not include the body read.

| Field | Meaning |
| --- | --- |
| `reused` | Every leg reused a pooled connection |
| `connect_ms` | DNS, TCP, TLS, and HTTP/2 preface over the legs that opened a connection; `None` when every leg was warm |
| `send_ms` | Request sent to response head |
| `total_ms` | Connect plus send |

`tls()` returns a `TlsInfo` with the negotiated `version`, the `cipher`, and
the peer certificate as DER (`peer_cert_der`). `audit()` returns the
fingerprints the session presents when it was built with `.audit(true)`, and
`None` otherwise. See [Fingerprints](fingerprints.md#audit-a-session).

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new().with_proxy("http://user:pass@proxy.example:8080");
let resp = session.get("https://example.com/").await?;
let t = resp.timing();
println!(
    "{} reused={} total={}ms tls={:?}",
    resp.proxy().unwrap_or("direct"),
    t.reused,
    t.total_ms,
    resp.tls().and_then(|tls| tls.version.as_deref()),
);
# Ok(())
# }
```

## Detect a block page

`block()` returns a `BlockSignal` when the response matches a built-in rule
for a bot challenge, a captcha, or a block page. `BlockRules` adds your own
rules. See [Crawling](crawling.md#detect-a-block-page).

## Print a response

`{:?}` prints the status, version, URL, redirect chain, headers, trailers,
request headers, and timing, but not the body. URLs print with the password
and the query hidden. `Authorization`, `Proxy-Authorization`, `Cookie`, and
`Set-Cookie` values print as `***`.
