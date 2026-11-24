# Cancellation

Rust has no `AbortController`. To cancel an in-flight request, stop polling
its future and drop it. A timeout, `tokio::select!`, or `JoinHandle::abort`
usually does this for you. When you drop the future, a `.stream()` response,
or its `bytes_stream()`, Leyline cleans up the connection or stream.

## Cancel from your code

When you only need a time bound, use a timeout. `.timeout(duration)` bounds
the request up to the response headers. `Timeout::body` bounds a streamed
body or a download. See [Retries and timeouts](retries-and-timeouts.md#the-timeouts).

To cancel on an event such as Ctrl-C or a shutdown channel, race the request
in `tokio::select!`. The losing branch is dropped. Enable the Tokio features
`macros`, `rt-multi-thread`, and `signal` or `sync`:

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

A dropped future stops running at once, so it never returns an `Err`. A
`Trace` hook does not receive a `leyline::trace::Event::Summary` for it.

## Drop a request before send returns

A request without `.stream()` reads the whole body before `send()` returns.
Dropping its future stops both the request and the body read.

| Protocol | Effect on the wire |
| --- | --- |
| HTTP/1.1 | Closes the connection instead of returning it to the pool and releases the per-host slot. The server may already have received part or all of the request |
| HTTP/2 | Queued requests are never sent; sent requests receive `RST_STREAM` (`CANCEL`) within ~100 ms while the connection stays pooled |
| HTTP/3 | Queued requests are never sent; open streams cancel within ~100 ms while the connection stays pooled |

- An in-progress TCP/TLS dial finishes in the background so the next request can reuse the warm connection.
- A request waiting on a `HostLimits` slot leaves the queue; one holding a slot releases it.
- A `Response::download_to` dropped before its commit starts removes its temporary `.part` file. The target path stays untouched. Once the commit starts, the rename completes.
- A resumable `RequestBuilder::download_to` keeps its `<name>.leyline.part` checkpoint, so the next call can resume. See [Streaming](streaming.md#download-a-file).
- Leyline stops reading a streaming request body. On HTTP/2 and HTTP/3 it resets the stream with `CANCEL`.
- Leyline reads your body stream only once the request is sent. It holds at most 256 KiB that the server's flow control has not yet accepted.

## Drop a streamed response

With `.stream()`, `send()` returns as soon as the response headers arrive.
Drop the `Response` or its `bytes_stream()` to stop reading early. This is a
cheap way to skip a body after you check `Content-Type` or read a prefix:

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
