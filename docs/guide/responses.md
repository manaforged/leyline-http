# Responses

A `Response` carries the status, the headers, the body, and the metadata
Leyline collected while sending: the final URL, the redirect chain, the proxy,
the timing, and the TLS facts. Read the metadata first, because every body
call consumes the response.

## Status and headers

`status()` returns an `http::StatusCode`, with `is_success`,
`is_redirection`, `is_client_error`, and `is_server_error`. `version()`
returns the `http::Version`: `HTTP_11`, `HTTP_2`, or `HTTP_3`.

`headers()` returns the `http::HeaderMap`, duplicates included. Iteration
does not follow wire order; `get_all` yields the values of one name in the
order they arrived. `header(name)` returns the first value as `&str`, or
`None` when it is missing or holds a byte outside visible ASCII. Read
`headers()` for the raw bytes.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session.get("https://example.com/").send().await?;
if resp.status().is_success() {
    println!("{} {:?}", resp.status().as_u16(), resp.version());
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
order, after the request kind, the cookie jar, and your headers were merged. It
needs `.audit(true)` on the session and is empty otherwise. The values are
prepared before dispatch, not captured from the transport.

`trailers()` iterates trailing headers in wire order. Buffered HTTP/2 and
HTTP/3 responses carry them; a streamed response and HTTP/1.1 yield none.

### Relay headers

`relay_headers()` returns the headers without the hop-by-hop ones:
`connection`, each header that `connection` names, `keep-alive`,
`proxy-connection`, `proxy-authenticate`, `proxy-authorization`, `te`, `trailer`, `transfer-encoding`, and `upgrade` (RFC
9110, section 7.6.1). When Leyline decodes the body, it also drops
`content-encoding` and `content-length`, so the headers match what every
sink returns. When it does not decode, because the request set
`decode(false)` or the session does not offer the coding, the body arrives as
sent and `content-encoding` stays. `proxy-authenticate` and
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
| `text_with_charset(label).await` | `Result<String>` | Uses `label` when the response declares no charset. Feature `charset` |
| `bytes().await` | `Result<bytes::Bytes>` | The decoded body |
| `json::<T>().await` | `Result<T>` | Deserializes with serde. A failure is `Kind::Json`, in `Kind::Decode` |
| `bytes_stream()` | `Result<impl Stream<Item = Result<Bytes>>>` | The decoded body as a stream |
| `write_to(writer).await` | `Result<u64>` | Writes the decoded body |
| `download_to(path).await` | `Result<u64>` | Saves the decoded body to a file; see [Streaming](streaming.md#download-a-file) |
| `read_until(limit, done).await` | `Result<(Vec<u8>, bool)>` | Stops early on a predicate or a byte limit; the `bool` is `true` when the predicate matched; see [Streaming](streaming.md#stop-at-a-marker) |

[Streaming](streaming.md) covers the streaming calls and downloads.

`text()` replaces invalid sequences with U+FFFD, and a leading byte order mark
overrides the declared charset. Without the `charset` feature, on by default,
`text()` uses lossy UTF-8.

### Size cap and content coding

`max_body_size` (100 MiB by default) caps every body you read: `text`,
`bytes`, `json`, `bytes_stream`, `write_to`, and `download_to`, on every
protocol. It counts decoded bytes. A body over the cap fails with
`Kind::BodyLimit`, and the message names `max_body_size`. Set it on the
session, or on one request, where it replaces the session's value for a
`.stream()` response:

```rust,no_run
use leyline::Session;

# async fn run() -> leyline::Result<()> {
let session = Session::builder().max_body_size(8 * 1024 * 1024).build()?;
let big = session
    .get("https://example.com/export.csv")
    .max_body_size(1024 * 1024 * 1024)
    .stream()
    .send()
    .await?;
# let _ = big;
# Ok(())
# }
```

A session offers and decodes gzip, deflate, br, and zstd, those that are
compiled in. `accept_encoding([..])` picks the codings it offers, and
`accept_encoding([])` offers none. Every sink decodes the codings the session
offers. `RequestBuilder::decode(false)` turns decoding off for one request:
every sink then returns the bytes as received, and the request still sends
the same `accept-encoding`. Repeated `Content-Encoding` fields combine into
one list. More than four codings fail with `Kind::Decode`. A coding the
session does not offer comes back as received.

## Final URL and redirects

`url()` returns the final URL after redirects. `redirect_chain()` returns
each URL that answered with a redirect Leyline followed, in order, without
the final URL and without user name or password. It is empty when nothing
redirected. `attempts()` returns the number of transport attempts, 1 when no
retry ran.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session.get("https://example.com/old").send().await?;
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

`RequestBuilder::pages()` turns a request into a stream of responses that
follows `Link: rel="next"`. Read it with `futures_util::StreamExt::next`,
and cap it with `take(n)`. Each follow-up is a `GET` with the first request's
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
use futures_util::StreamExt;

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
the jar refused. Read the jar with `session.cookies().get(&url, name)`.
See [Cookies](cookies.md).

## Turn a status into an error

A 4xx or 5xx response is `Ok`. `Response::error_for_status()` returns a
`Kind::Status` error for a status of 400 or more, with the status, the
headers, and the final URL. It borrows the response, so you can still read
the body of an error.

`RequestBuilder::error_for_status()` checks the final response after the
retry policy is done, and keeps the start of the body in the error:

| Limit | Default | Set with |
| --- | --- | --- |
| Body bytes kept, after decoding | 64 KiB, and never more than `max_body_size` | `SessionBuilder::max_error_body_size` |
| Time to read that body | 10 s, and never past the `total` timeout | `Timeout::error_body` |

Leyline decides after the status line. A response below 400 is read in full
as usual. An error response is read only up to these limits, so a large or
slow error body cannot hold the request to the `body` timeout or exceed
`max_body_size`. Decompression stops at the byte limit, so a small
compressed body cannot expand past it.

The error-body timeout starts when the response head arrives, and each error
response gets one window, however its body is read. If the read fails or the
window ends first, the error keeps the bytes decoded so far, or no body if
none arrived, with the status and headers. The error's headers drop
`content-encoding` when Leyline decoded the body, and `content-length` when
the kept body is not the full declared length.

`max_error_body_size` counts decoded bytes. Over HTTP/1.1, an error body that
declares a length of `max_error_body_size` or less is read with the response. If
it arrives within the window, the connection goes back to the pool;
otherwise Leyline closes it. A larger or unframed error body is read only
until `max_error_body_size` decoded bytes. If the rest of a fixed-length error
body is no larger than `max_error_body_size`, Leyline reads and discards it in the
background, within the pool's idle timeout, so the connection can be reused;
this never extends the error-body window. Otherwise it closes the connection.
Over HTTP/2 and HTTP/3, the stream is reset and the connection stays open.
`download_to` after `error_for_status()` uses the same limits for a
status error.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session.get("https://example.com/api").send().await?;
if let Err(e) = resp.error_for_status() {
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
connection. Under a proxy list it names the proxy of the attempt that
produced the response.

### Timing

These calls report time summed across redirect legs, on every protocol. They
do not include the body read.

| Call | Returns | Meaning |
| --- | --- | --- |
| `reused()` | `bool` | Every leg reused a pooled connection |
| `connect_time()` | `Option<Duration>` | DNS, TCP, TLS, and HTTP/2 preface over the legs that opened a connection; `None` when every leg was warm |
| `send_time()` | `Duration` | Request sent to response head |
| `elapsed()` | `Duration` | Connect plus send: the time to the response head |

`tls_version()` and `tls_cipher()` return the negotiated version and cipher,
and `peer_certificate()` the peer certificate as DER. `fingerprint()` returns the
fingerprints the session presents when it was built with `.audit(true)`, and
`None` otherwise. See [Fingerprints](fingerprints.md#audit-a-session).

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .proxy("http://user:pass@proxy.example:8080")
    .build()?;
let resp = session.get("https://example.com/").send().await?;
println!(
    "{} reused={} elapsed={:?} tls={:?}",
    resp.proxy().unwrap_or("direct"),
    resp.reused(),
    resp.elapsed(),
    resp.tls_version(),
);
# Ok(())
# }
```

## Classify a block response

`block()` returns a `Block` when the response matches a block rule:
`Challenge`, `Captcha`, or `Denied`, each with the vendor name. See [Crawling](crawling.md#classify-a-block-response).

## Print a response

`{:?}` prints the status, version, URL, redirect chain, headers, trailers,
request headers, and timing, but not the body. URLs print with the password
and the query hidden. `Authorization`, `Proxy-Authorization`, `Cookie`, and
`Set-Cookie` values print as `***`.
