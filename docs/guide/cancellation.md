# Cancellation

Rust has no `AbortController`: you cancel an in-flight request by stopping
polling and dropping its future — usually through a timeout, `tokio::select!`
(for Ctrl-C or a shutdown channel), or `JoinHandle::abort`. Dropping the future,
the `Response` of a `.stream()` request, or its `bytes_stream()` automatically
tells Leyline to clean up the connection or stream.

## Cancel from your code

Prefer a timeout when you only need a time bound: `.timeout(duration)` bounds
the request up to the response headers, and `Timeout::body` bounds a streamed
body or download (see [Retries and timeouts](retries-and-timeouts.md#the-timeouts)).

To cancel on an external event such as Ctrl-C or a shutdown channel, race the
request in `tokio::select!` (the losing branch is dropped; enable Tokio
features `"macros"`, `"rt-multi-thread"`, and `"signal"` or `"sync"`):

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
tokio::select! {
    result = session.get("https://example.com/archive.tar").download_to("archive.tar") => {
        println!("downloaded {} bytes", result?);
    }
    _ = tokio::signal::ctrl_c() => {
        println!("cancelled; partial file kept for next resume");
    }
}
# Ok(())
# }
```

To stop every request on a session and its clones at once, call
`Session::shutdown()`; see [Sessions](sessions.md#stop-a-session).

Because a dropped future stops running immediately, it never returns an `Err`
(and tracing listeners will not receive an `Event::Summary`).

## Drop a request before the response headers

A request without `.stream()` reads the whole body before `send()` returns, so
dropping its future stops both the request and the body read.

| Protocol | Effect on the wire |
| --- | --- |
| HTTP/1.1 | Closes the connection instead of returning it to the pool and releases the per-host slot. The server may already have received part or all of the request |
| HTTP/2 | Queued requests are never sent; sent requests receive `RST_STREAM` (`CANCEL`) within ~100 ms while the connection stays pooled |
| HTTP/3 | Queued requests are never sent; open streams cancel within ~100 ms while the connection stays pooled |

- An in-progress TCP/TLS dial finishes in the background so the next request can reuse the warm connection.
- A request waiting on a `HostLimits` slot leaves the queue; one holding a slot releases it.
- A `Response::download_to` dropped before commit starts removes its temporary `.part` file and leaves the target path untouched; once the blocking commit starts, the rename completes. A resumable `RequestBuilder::download_to` keeps its `<name>.leyline.part` checkpoint so the next call can resume. See [Streaming](streaming.md#download-a-file).
- A streaming request body stops being polled (and resets the H2/H3 stream with `CANCEL`). Leyline starts polling only after the request begins and buffers at most 256 KiB ahead of peer flow control.

## Drop a streamed response

With `.stream()`, `send()` returns as soon as the response headers arrive. Drop
the `Response` or its `bytes_stream()` to stop reading early — a cheap way to
skip a body after checking `Content-Type` or reading a prefix:

| Protocol | Effect on the wire |
| --- | --- |
| HTTP/1.1 | Closes the connection (reading to EOF returns it to the pool) |
| HTTP/2 | Resets only that stream; the connection stays pooled |
| HTTP/3 | Resets only that stream; the connection stays pooled ([HTTP/3](http3.md) lists reset codes) |

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session.get("https://example.com/file").stream().send().await?;
if !resp
    .header("content-type")
    .is_some_and(|value| value.starts_with("text/html"))
{
    return Ok(());
}
let mut body = resp.bytes_stream()?;
let mut seen = 0usize;
while let Some(chunk) = body.next().await {
    seen += chunk?.len();
    if seen > 1024 * 1024 {
        break;
    }
}
# Ok(())
# }
```
