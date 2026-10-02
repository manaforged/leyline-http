# Cancellation

To cancel a request, drop it: the request future, the `Response` of a
`.stream()` request, or its `BodyStream`. Leyline then stops the request and
frees what it holds. A timeout, `tokio::select!`, and `JoinHandle::abort` all
cancel by dropping. What happens on the wire depends on the protocol and on
how far the request got.

## Drop a request before the response head

A request without `.stream()` reads the whole body before `send()` returns, so
dropping its future also stops the body read.

| Protocol | Effect |
| --- | --- |
| HTTP/1.1 | Leyline closes the connection and does not return it to the pool. The server can already have part or all of the request. The per-host connection slot is released |
| HTTP/2 | A request that waits for a stream slot is never sent. A sent request gets `RST_STREAM` with `CANCEL` within about 100 ms. The connection stays in the pool |
| HTTP/3 | A queued request is never sent, and an open stream is cancelled within about 100 ms. The connection stays in the pool |

- A connection that Leyline is dialing for the request keeps dialing. When
  it is ready, the pool keeps it for the next request.
- A request that waits for a `HostLimits` slot leaves the wait, and one that
  holds a slot gives it back. See [Crawling](crawling.md#limit-each-host).
- A dropped `download` removes its temporary `.part` file and leaves the
  target path as it was.
- A request with a streaming body stops polling the body. On HTTP/2 and
  HTTP/3, Leyline resets the stream with `CANCEL`. It polls the body only
  after the request starts and buffers at most 256 KiB ahead of the peer's
  flow-control window.

## Drop a streamed response

With `.stream()`, `send()` returns at the response head. Drop the `Response`
or its `BodyStream` to stop the body. A drop is a cheap way to skip a body
you do not want.

| Protocol | Effect |
| --- | --- |
| HTTP/1.1 | Leyline closes the connection and does not return it to the pool, so the next request to the host opens a new one. A body read to the end returns the connection to the pool |
| HTTP/2 | Leyline resets the stream. The connection stays in the pool |
| HTTP/3 | Leyline resets the stream. The connection stays in the pool. [HTTP/3](http3.md) lists the reset codes |

```rust,no_run
use futures_util::StreamExt;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session.get("https://example.com/file").stream().await?;
if !resp
    .header("content-type")
    .is_some_and(|value| value.starts_with("text/html"))
{
    return Ok(());
}
let mut body = resp.into_stream()?;
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

## Cancel from your code

Prefer a timeout: `.timeout(duration)` bounds the request, and
`TimeoutConfig::body` bounds a streamed body. See
[Retries and timeouts](retries-and-timeouts.md#the-timeouts). To stop a
request on another event, such as a shutdown signal, use `tokio::select!`; the
branch that loses is dropped.

```rust,no_run
use tokio::sync::oneshot;

# async fn run(shutdown: oneshot::Receiver<()>) -> leyline::Result<()> {
let session = leyline::Session::new();
tokio::select! {
    result = session.get("https://example.com/feed").send() => {
        println!("{}", result?.status());
    }
    _ = shutdown => println!("shutting down"),
}
# Ok(())
# }
```

To stop every request of a session and its clones at once, call
`Session::shutdown()`; see [Sessions](sessions.md#stop-a-session).

A dropped request future returns no error, because nothing polls it, and the
`Trace::summary` hook does not fire for it. See
[Logging and tracing](logging.md).
