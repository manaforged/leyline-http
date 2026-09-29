# Streaming

Stream when a body is larger than you want in memory, or when you want to act
on the first bytes before the last ones arrive. Streaming needs no feature.
The examples in this chapter also need `bytes`, `futures-util`, and
`tokio-util` with the `io` feature in your manifest.

## Stream a request body

`Body::stream(s, len)` takes any `Stream` of `io::Result<Bytes>` and an
`Option<u64>` byte count.

Give the length whenever you know it. With a length, Leyline sends
`Content-Length`. Without one, the body is sent with no declared length, which
some origins reject.

```rust,no_run
use leyline::Body;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let file = tokio::fs::File::open("upload.bin").await?;
let len = tokio::fs::metadata("upload.bin").await?.len();
let body = Body::stream(tokio_util::io::ReaderStream::new(file), Some(len));

let resp = session
    .post("https://example.com/upload")
    .header("content-type", "application/octet-stream")
    .body(body)
    .await?;
println!("{}", resp.status());
# Ok(())
# }
```

`Body::len_hint()` reports the declared length: the buffer size for a buffered
body, `Some(0)` for an empty one, and the hint you supplied for a stream.

## Stream a response body

Call `.stream()` on the request builder. The response then arrives as soon as
the headers do, and `into_stream()` hands you the body as a `BodyStream`, which
implements `futures_util::Stream<Item = io::Result<Bytes>>`.

```rust,no_run
use futures_util::StreamExt;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session.get("https://example.com/big").stream().await?;

let mut body = resp.into_stream()?;
let mut total = 0u64;
while let Some(chunk) = body.next().await {
    total += chunk?.len() as u64;
}
println!("{total} bytes");
# Ok(())
# }
```

`copy_to(writer)` does the same loop for you and returns the byte count. On a
response from `.stream()`, `into_stream` and `copy_to` hand back the
content-encoded bytes as they arrive. `bytes()`, `text()`, and `json()` decode
compression when they drain a body. When you need decoded bytes from a stream,
request identity encoding or use `read_until` (see
[Stop at a marker](#stop-at-a-marker)).

A `.stream()` body has no size cap on HTTP/1.1, HTTP/2, or HTTP/3, and
`into_stream` and `copy_to` add none. You read the chunks, so you decide how
much stays in memory. `CompressionConfig::max_body_size` (100 MiB by default)
caps what Leyline holds in memory: the bytes that `bytes()`, `text()`, and
`json()` collect, and the decoded output of a compressed body. A body over the
cap fails with `Kind::Body`, and the message names `max_body_size`.

You do not have to stream it yourself. `bytes().await`, `text().await`, and
`json().await` drain a streaming body for you and decompress it, so they work
in both modes. They consume the response. Take the stream or drain it, not
both: `into_stream()` consumes the response, so nothing is left to read after
it. A streaming response also carries no trailers.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let body = session.get("https://example.com/big").stream().await?.text().await?;
println!("{} bytes", body.len());
# Ok(())
# }
```

## Stop at a marker

`read_until(limit, done)` reads a streamed body, decodes gzip, Brotli, zstd,
and deflate as the chunks arrive, and stops when you tell it to. Use it when
the value you need sits near the top of a large page.

After each chunk, Leyline calls `done(body, from)`. `body` is every decoded
byte so far. `from` is the offset where the newest chunk starts. Return `true`
to stop. The read also stops when `body` reaches `limit` decoded bytes or the
stream ends. The call returns the decoded prefix and drops the connection's
remaining body.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let marker = b"</head>";
let resp = session.get("https://example.com/big").stream().await?;
let head = resp
    .read_until(256 * 1024, |body, from| {
        let start = from.saturating_sub(marker.len());
        body[start..].windows(marker.len()).any(|w| w == marker)
    })
    .await?;
println!("{} decoded bytes", head.len());
# Ok(())
# }
```

Scan from `from` minus the marker length, as the example does. A marker can
straddle two chunks, and a scan of the whole body on every chunk costs
quadratic time on a large page.

`read_until` consumes the response. Read the status, headers, `timing()`, and
`audit()` before you call it.

An unknown `Content-Encoding`, or one that the session's `CompressionConfig`
turns off, is not decoded, and `read_until` returns the raw bytes. Corrupt
compressed data fails with `Kind::Decode`. Decoded output over `max_body_size`
fails with `Kind::Body`.

## Back-pressure

A streamed response body reaches you through a bounded channel, so a slow
reader slows the sender instead of growing a buffer.

On HTTP/1.1, the task that reads the socket waits while the channel of 16
chunks is full. The receive buffer of the socket then fills, and TCP flow
control slows the peer.

On HTTP/2, the response channel holds 32 chunks. When you stop polling the
`BodyStream`, the driver keeps reading the connection, so other streams keep
going. It queues the data of the stalled stream and sends no window updates for
that stream while the queue is not empty. The queue cannot grow past the
receive window of the stream, so the peer stops sending on it.

On HTTP/3, Leyline stops reading the QUIC stream while the channel of 32
chunks is full, and QUIC flow control slows the peer.

The `read` timeout applies per chunk, not to the whole body. It measures the
gap between chunks, so a slow but steady download does not trip it. A gap
longer than the timeout yields an `io::ErrorKind::TimedOut` error from the
stream. See [Retries and timeouts](retries-and-timeouts.md).

## When a body cannot be replayed

A buffered body can be sent again. A streaming body cannot: the stream has
already been consumed by the first attempt. That has two consequences.

**Redirects.** A redirect either changes the request to a GET or replays it.
After a 301 or 302, only a POST becomes a GET with an empty body. After a 303,
every method except HEAD does. A request that becomes a GET follows normally.
Every other request keeps its method and its body, so a 307 or 308, or a 302
for a PUT, must replay the body. A streaming body cannot be replayed. Leyline
then stops and returns `Kind::Redirect`, telling you to buffer the body before
sending or to set `RedirectPolicy::none()`.

**Retries.** The retry loop checks the body before it sleeps. A streaming body
is not retryable, so the policy is skipped and the first outcome is returned,
whatever the policy says.

If you need retries, or a request that a redirect can replay, buffer the body
yourself and send it as `Bytes`.

## Next

Read [Redirects](redirects.md) to see what a redirect changes in the next
request.
