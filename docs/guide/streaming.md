# Streaming

Stream when a body is larger than you want in memory, or when you want to act
on the first bytes before the last ones arrive. This chapter covers request
bodies, response bodies, downloads, and flow control. Streaming needs no
feature. The examples also need `bytes`, `futures-util`, and `tokio-util` with
the `io` feature.

## Stream a request body

`Body::stream(s, len)` takes any `Stream` of `io::Result<Bytes>` and an
`Option<u64>` length. Give the length whenever you know it: Leyline then
sends `Content-Length` on every protocol. Without it, HTTP/1.1 sends
`Transfer-Encoding: chunked` and HTTP/2 and HTTP/3 declare no length, which
some origins reject. `Body::len_hint()` reports the declared length.

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

When your stream returns an error, the request fails with `Kind::Body`, and
`Error::io()` returns your error. Leyline does not resend, because a stream
cannot be replayed. An HTTP/2 or HTTP/3 connection resets only that stream
and stays open. An HTTP/1.1 connection closes, because it holds part of the
request.

### When a body cannot be replayed

A streamed body is consumed by the first attempt, so:

- **Redirects.** A redirect that turns the request into a GET follows
  normally. A redirect that keeps the method and body, such as a 307 or 308,
  fails with `Kind::Redirect`. See [Redirects](redirects.md).
- **Retries.** The retry policy is skipped, and the first outcome is
  returned.

For retries or replayable redirects, buffer the body and send it as `Bytes`.

## Stream a response body

Call `.stream()` on the request builder. The response arrives with the
headers, and `into_stream()` returns the body as a `BodyStream`, a
`futures_util::Stream<Item = io::Result<Bytes>>`.

```rust,no_run
use futures_util::StreamExt;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session.get("https://example.com/big").stream().await?;
let mut body = resp.into_decoded_stream(Some(64 * 1024 * 1024))?;
let mut total = 0u64;
while let Some(chunk) = body.next().await {
    total += chunk?.len() as u64;
}
println!("{total} bytes");
# Ok(())
# }
```

`text()`, `bytes()`, and `json()` also work on a `.stream()` response: they
drain and decode it. A streamed response carries no trailers.

### Decoded or raw

| Calls | Bytes | Cap | Use it to |
| --- | --- | --- | --- |
| `into_decoded_stream(limit)`, `copy_decoded_to(writer, limit)` | Content coding removed | The smaller of `limit` and `max_body_size` | Save a file, parse the body |
| `into_stream()`, `copy_to(writer)` | As sent, still encoded | None | Pass the body on with its `content-encoding`, as a proxy does |

The decoded calls decode chunk by chunk and work on a buffered response too.
`limit: None` means `max_body_size`. A body of exactly `limit` bytes passes. A
longer one fails with `Error::is_body_limit()` true, and the chunk that
crosses the limit is not written. A coding that `CompressionConfig` turns off
passes through still encoded. See [Responses](responses.md#bodies).

## Download a file

`RequestBuilder::download(path, limit)` sends the request with `.stream()`
and saves the decoded body to `path`. It returns the number of bytes written.
`Response::download_to(path, limit)` does the same for a response you hold.

| Step | What happens |
| --- | --- |
| Status | With `download`, a status of 400 or more is a `Kind::Status` error with the status, the URL, the headers, and the start of the body, within the [status-error limits](responses.md#turn-a-status-into-an-error). Nothing is written. `download_to` does not check the status; call `error_for_status()` first |
| Write | The decoded body goes to a temporary file `.<name>.<16 hex>.part` in the same directory |
| Limit | The smaller of `limit` and `max_body_size`, in decoded bytes. `content_length()` is the encoded size |
| Commit | Leyline flushes and syncs the file, renames it to `path`, and on Unix syncs the directory. When `path` already exists, the new file keeps its permission bits, without setuid, setgid, or sticky bits. |
| Error or drop | Leyline removes the temporary file. `path` does not exist, or keeps its old content |

A path with no file name fails with `Kind::Request`.

### Bound the whole download

For a `.stream()` request, `total` covers the request up to the response
head. Two timeouts bound the body:

- `read` limits the gap between two chunks. It stops a server that goes
  quiet. A gap past it is an `io::ErrorKind::TimedOut` error from the stream.
- `body` limits the whole body, from its first read. It stops a server that sends
  slowly but never stops. A plain `Duration` passed to `.timeout` sets only
  `total`, which ends at the head of a streamed response.

```rust,no_run
use std::time::Duration;

use leyline::TimeoutConfig;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
match session
    .get("https://example.com/image.iso")
    .timeout(
        TimeoutConfig::new()
            .read(Duration::from_secs(30))
            .body(Duration::from_secs(600)),
    )
    .download("image.iso", Some(8 * 1024 * 1024 * 1024))
    .await
{
    Ok(n) => println!("{n} bytes"),
    Err(e) if e.is_body_limit() => eprintln!("too large: {e}"),
    Err(e) => return Err(e),
}
# Ok(())
# }
```

See [Retries and timeouts](retries-and-timeouts.md#the-timeouts). To stop a
download from your code, drop the future; see
[Cancellation](cancellation.md).

## Stop at a marker

`read_until(limit, done)` reads and decodes a body as the chunks arrive and
stops when you tell it to. Use it when the value you need sits near the top
of a large page. After each chunk, Leyline calls `done(body, from)`: `body` is
every decoded byte so far, and `from` is where the newest chunk starts.
Return `true` to stop. The read also stops at `limit` decoded bytes or at the
end of the stream. The call returns the decoded prefix and drops the rest of
the body.

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

Scan from `from` minus the marker length, as the example does: a marker can
straddle two chunks, and a scan of the whole body on every chunk costs
quadratic time.

Reaching `limit` is not an error, and the result does not say the body was
longer; to enforce a size limit, use the decoded calls. An unknown or
turned-off `Content-Encoding` is not decoded. Corrupt compressed data fails
with `Kind::Decode`, and output over `max_body_size` with `Kind::Body`.

## Back-pressure

A streamed response body reaches you through a bounded channel, so a slow
reader slows the sender instead of growing a buffer.

| Protocol | Channel | When you stop reading |
| --- | --- | --- |
| HTTP/1.1 | 16 chunks | The socket reader waits, the socket buffer fills, and TCP flow control slows the peer |
| HTTP/2 | 32 chunks | The driver keeps serving other streams. It queues the stalled stream's data and sends no window update for it, so the queue stays within the stream's receive window |
| HTTP/3 | 32 chunks | Leyline stops reading the QUIC stream, and QUIC flow control slows the peer |

Dropping a `BodyStream` before its end stops the transfer. See
[Cancellation](cancellation.md) for what happens to the connection.
