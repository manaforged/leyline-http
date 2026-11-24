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
body, it removes `Content-Encoding` and `Content-Length` from the headers. So
`content_length()` returns `None` for a decoded buffered body. A `.stream()`
response keeps the wire values.

`request_headers()` lists the headers the session prepared to send, in send
order, after the request kind, the cookie jar, and your headers were merged. It
needs `.audit(true)` on the session and is empty otherwise. The values are
prepared before dispatch, not captured from the transport.

`trailers()` iterates the trailing headers. Buffered responses carry them on
HTTP/1.1 (chunked bodies), HTTP/2, and HTTP/3. A `.stream()` response yields
none.

### Relay headers

`relay_headers()` returns a header map ready to forward downstream:

- Strips RFC 9110 §7.6.1 hop-by-hop headers: `connection`, every header listed in `connection`, `keep-alive`, `proxy-connection`, `proxy-authenticate`, `proxy-authorization`, `te`, `trailer`, `transfer-encoding`, and `upgrade`.
- When Leyline decodes the body, also strips `content-encoding` and `content-length` so the headers match the decoded body returned by `text()`, `bytes()`, `bytes_stream()`, and `download_to()`.
- When decoding is disabled (`decode(false)`) or the coding is not offered, `content-encoding` stays intact.

See [Service integration](service-integration.md#relay-an-upstream-body).

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
| `json::<T>().await` | `Result<T>` | Deserializes with serde. A failure is `Kind::Json` |
| `bytes_stream()` | `Result<Body>` | The decoded body as a stream of `Result<Bytes>` |
| `write_to(writer).await` | `Result<u64>` | Writes the decoded body |
| `download_to(path).await` | `Result<u64>` | Saves the decoded body to a file; see [Streaming](streaming.md#download-a-file) |
| `read_until(limit, done).await` | `Result<(Vec<u8>, bool)>` | Stops early on a predicate or a byte limit; the `bool` is `true` when the predicate matched; see [Streaming](streaming.md#stop-at-a-marker) |

[Streaming](streaming.md) covers the streaming calls and downloads.

`text()` replaces invalid sequences with U+FFFD, and a leading byte order mark
overrides the declared charset. The `charset` feature is on by default. With
it off, `text()` ignores the declared charset and decodes as lossy UTF-8.

### Size cap and content coding

`max_body_size(impl Into<Option<usize>>)` (100 MiB by default) caps decoded
bytes across every body method (`text`, `bytes`, `json`, `bytes_stream`,
`write_to`, `download_to`) on every protocol, failing with `Kind::BodyLimit`.
Set it on the session or on one request (applies to both buffered and
`.stream()` responses); pass `None` on either builder to remove the cap:

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

- A session offers and decodes every compiled-in coding (`gzip`, `deflate`, `br`, `zstd`). `accept_encoding([..])` restricts the list, and `accept_encoding([])` offers none.
- `RequestBuilder::decode(false)` skips decompression for one request and leaves `accept-encoding` unchanged. A coding the session did not offer also passes through raw.
- Repeated `Content-Encoding` headers combine into one list. More than four codings fail with `Kind::Decode`.

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

`links()` parses RFC 8288 `Link` headers (multiple headers, comma-separated
entries, quoted parameters, multi-value `rel`). Each `Link` exposes `url()`,
`rel()`, `params()`, and `has_rel(name)` (case-insensitive); relative URLs
resolve against `resp.url()`. For example, find the next page with
`resp.links().into_iter().find(|l| l.has_rel("next"))`.

`RequestBuilder::pages()` turns a request into a `futures_util::Stream` of
responses that follows `Link: rel="next"` automatically (add
`futures-util = "0.3"` to `Cargo.toml` and import `futures_util::StreamExt`):

- Read with `futures_util::StreamExt::next` and cap with `.take(n)`.
- Follow-ups use `GET` with the first request's headers, timeouts, retry policy, and `error_for_status()` setting.
- Stops when a page has no `next` link, returns an error, or repeats an already-visited URL (including final URLs after redirects).

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

To check the status on the request instead, call
`RequestBuilder::error_for_status()`. It also keeps the start of the error
body. [Errors](errors.md#status-codes-are-not-errors) covers both calls, the
body limits, and the error accessors.

## Proxy, timing, and TLS

`proxy()` returns the proxy that carried the response, with the password, or
a user name without a password, replaced by `***`. It is `None` for a direct
connection. Under a proxy list it names the proxy of the attempt that
produced the response.

`remote_addr()` returns the socket address the connection was opened to: the
server's resolved address on a direct connection, and the proxy's address
through a proxy. It is `None` when `from_cache()` is `true`.

### Timing

A leg is one request and its response; each followed redirect adds a leg.
These calls sum the time over all legs, on every protocol. They do not
include the body read.

| Call | Returns | Meaning |
| --- | --- | --- |
| `reused()` | `bool` | Every leg reused a pooled connection |
| `connect_time()` | `Option<Duration>` | DNS, TCP, TLS, and HTTP/2 preface over the legs that opened a connection; `None` when every leg reused one |
| `send_time()` | `Duration` | Request sent to response head |
| `elapsed()` | `Duration` | Connect plus send: the time to the response head |

`tls_version()` and `tls_cipher()` return the negotiated version and cipher,
`ech_accepted()` reports whether TLS 1.3 Encrypted Client Hello succeeded (see
[TLS trust](tls-trust.md#encrypt-sni-with-ech)), and `peer_certificate()`
returns the leaf certificate as DER. `fingerprint()` returns the fingerprints
the session presented when built with `.audit(true)` (`None` otherwise); see
[Fingerprints](fingerprints.md#audit-a-session).

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .proxy("http://user:pass@proxy.example:8080")
    .build()?;
let resp = session.get("https://example.com/").send().await?;
println!(
    "{} reused={} elapsed={:?} tls={:?}",
    resp.proxy().map(leyline::Url::as_str).unwrap_or("direct"),
    resp.reused(),
    resp.elapsed(),
    resp.tls_version(),
);
# Ok(())
# }
```

## Cache hits

`from_cache()` is `true` when the body came from the session's HTTP cache.
That covers a fresh entry, answered with no request, and a stale entry that
the server confirmed with a `304`. A session has a cache only after
`SessionBuilder::cache(bytes)`; see
[Cache responses](api-clients.md#cache-responses).

To make the session check with the server, send the request header
`cache-control: no-cache`. The header makes the session revalidate the stored
entry; it does not bypass the cache.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::builder().cache(1 << 20).build()?;
let first = session.get("https://example.com/doc").send().await?;
let second = session.get("https://example.com/doc").send().await?;
println!("{} {:?}", first.from_cache(), first.remote_addr());
println!("{} {:?}", second.from_cache(), second.remote_addr());
let checked = session
    .get("https://example.com/doc")
    .header("cache-control", "no-cache")
    .send().await?;
# drop(checked);
# Ok(())
# }
```

See [Cache responses](api-clients.md#cache-responses) for the complete cache
bypass rules (including non-`GET`, `.stream()`, `Range`/`If-Range`/conditional
headers, and per-request `.protocol(...)` overrides).

## Convert to an http::Response

`into_http()?` converts the response into an `http::Response<Body>` for code
that uses the `http` crate's types. The body streams decoded, the final `Url`
is in the extensions, and the headers that describe a decoded encoding are
left out. Send the request with `.stream()` so that the body is not buffered
first.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session.get("https://example.com/big").stream().send().await?;
let http = resp.into_http()?;
let final_url = http.extensions().get::<leyline::Url>().map(leyline::Url::as_str);
println!("{} {final_url:?}", http.status());
let mut body = http.into_body();
while let Some(chunk) = body.next().await {
    println!("{} bytes", chunk?.len());
}
# Ok(())
# }
```

`Body::next` returns the next chunk, an error that ends the body, or `None`
after the last chunk. See [Streaming](streaming.md#stream-a-response-body).

## Classify a block response

`block()` returns a `Block` when the response matches a block rule:
`Challenge`, `Captcha`, or `Denied`, each with the vendor name. See [Crawling](crawling.md#classify-a-block-response).

## Print a response

`{:?}` prints the status, version, URL, redirect chain, headers, trailers,
request headers, and timing, but not the body. URLs print with the password
and the query hidden. `Authorization`, `Proxy-Authorization`, `Cookie`, and
`Set-Cookie` values print as `***`.
