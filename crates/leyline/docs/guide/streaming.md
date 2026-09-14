# Streaming

Stream when a body is larger than you want in memory, or when you want to act
on the first bytes before the last ones arrive. The `stream` feature is on by
default. The examples in this chapter also need `bytes`, `futures-util`, and
`tokio-util` with the `io` feature in your manifest.

## Stream a request body

`Body::stream` takes any `Stream` of `io::Result<Bytes>`.
`Body::stream_with_length` takes the same stream plus an exact byte count.

Give the length whenever you know it. With a length, Leyline sends
`Content-Length`. Without one, the body is sent with no declared length, which
some origins reject.

```rust,no_run
use leyline::Body;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::chrome();
let file = tokio::fs::File::open("upload.bin").await?;
let len = tokio::fs::metadata("upload.bin").await?.len();
let body = Body::stream_with_length(tokio_util::io::ReaderStream::new(file), len);

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
`Body::is_stream()` tells you which kind you hold.

## Stream a response body

Call `.stream()` on the request builder. The response then arrives as soon as
the headers do, and `into_stream()` hands you the body as a `BodyStream`, which
implements `futures_util::Stream<Item = io::Result<Bytes>>`.

```rust,no_run
use futures_util::StreamExt;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::chrome();
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

`copy_to(writer)` and `download_to(path)` do the same loop for you and return
the byte count. All three stream helpers hand back the content-encoded bytes
as they arrive; `bytes()`, `text()`, and `json()` decode compression when they
drain a body. Request identity encoding, or decode the stream yourself, when
you need decoded bytes from a stream.

HTTP/1.1 streaming rejects bodies above 100 MiB, fixed-length, chunked, or
close-delimited, and HTTP/3 streaming rejects them per chunk. Only HTTP/2
streaming does not apply that cap; `download_to` does not change this.

You do not have to stream it yourself. `bytes().await`, `text().await`, and
`json().await` drain a streaming body for you, decompress it, and keep the
bytes for later calls, so they work in both modes. Draining honors the session
`read_timeout` per chunk and the same 100 MiB cap that buffered mode applies.
Take the stream or drain it, not both: `into_stream()` consumes the response,
so nothing is left to read after it. The sync accessors
`as_bytes()` and `as_text()` return `None` until the body is buffered. A
streaming response also carries no trailers.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::chrome();
let mut resp = session.get("https://example.com/big").stream().await?;
assert!(resp.as_bytes().is_none());
let body = resp.text().await?;
println!("{} bytes", body.len());
# Ok(())
# }
```

## Back-pressure

Both directions are bounded channels, so a slow consumer slows the producer
instead of growing a buffer.

On HTTP/2, the streaming request body channel and the streaming response body
channel each hold 32 chunks. When the reader stops polling, the channel fills,
the connection driver stops draining, and the peer stops receiving window
updates. On the response side, the effect is the same in reverse: stop polling
the `BodyStream` and the sender stops.

The `read` timeout applies per chunk, not to the whole body. It measures the
gap between chunks, so a slow but steady download does not trip it. A gap
longer than the timeout yields an `io::ErrorKind::TimedOut` error from the
stream. See [Retries and timeouts](retries-and-timeouts.md).

## When a body cannot be replayed

A buffered body can be sent again. A streaming body cannot: the stream has
already been consumed by the first attempt. That has two consequences.

**Redirects.** A 301, 302, or 303 turns the request into a GET with an empty
body, so it follows normally. A 307 or 308 must replay the original body. With
a streaming body Leyline stops and returns `Kind::Redirect`, telling you to
buffer the body before sending or to set `max_redirects(0)`.

**Retries.** The retry loop checks the body before it sleeps. A streaming body
is not retryable, so the policy is skipped and the first outcome is returned,
whatever the policy says.

If you need retries or a 307-safe request, buffer the body yourself and send it
as `Bytes`.

## Next

Read [Retries and timeouts](retries-and-timeouts.md).
