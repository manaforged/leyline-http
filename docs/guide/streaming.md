# Streaming

Stream when a body is larger than you want in memory, or when you want to act
on the first bytes before the last ones arrive. This chapter covers resumable
file downloads, streaming request and response bodies, `read_until`, and
server-sent events (SSE).

Streaming needs no extra Cargo feature. `Body::next()`, `Body::file()`, and
`Events::next()` need no extra crates; `tokio` feature `"fs"` is used for
`tokio::fs::File`, and `bytes`, `futures-util`, and `tokio-util` (feature `"io"`)
are only for custom `Body::stream`, `StreamReader`, and `RequestBuilder::pages()`.

## Download a file

Call `download_to(path)` directly on `RequestBuilder` (before `send`) for
resumable, verified downloads that survive dropped connections and reject
`4xx`/`5xx` statuses automatically.

For large files, clear the default 300 s `Timeout::total` with `.total(None)`,
bound the transfer with `Timeout::read` and `Timeout::body`, and raise or
remove (`None`) the default 100 MiB `max_body_size` cap:

```rust,no_run
use std::time::Duration;

use leyline::{Session, Timeout};

# async fn run() -> leyline::Result<()> {
let session = Session::new();
let bytes = session
    .get("https://example.com/archive.tar")
    .timeout(
        Timeout::new()
            .total(None)
            .read(Duration::from_secs(30))
            .body(Duration::from_secs(600)),
    )
    .max_body_size(None)
    .retry(3)
    .progress(|written: u64, total: Option<u64>| {
        println!("{written} of {total:?} bytes");
    })
    .download_to("archive.tar")
    .await?;
println!("saved {bytes} bytes");
# Ok(())
# }
```

- **How resume works.** Accepts `GET` without a body or caller `Range` header and requests uncompressed (`identity`) bytes so byte offsets stay exact. Resumes using `Range` + `If-Range` when the server returned a strong `ETag` (such as `"abc123"`, not a weak `W/"..."` tag) and the final URL, ETag, byte range, and total length all agree; otherwise restarts cleanly on `200`.
- **Retries and checkpoints.** Checkpoints sync to disk after at least 1 MiB **and** 1 second since the last checkpoint, and on a reported body error. Under `.retry(n)`, dropped transfers retry automatically from the saved checkpoint (remember `Timeout::total` still spans retries unless cleared or raised); repeating `download_to(path)` later resumes the same checkpoint.
- **Verification and progress.** `.sha256([u8; 32])` verifies the complete file (including any resumed prefix) before atomic rename. `.resume(false)` ignores saved partial files. `.progress` takes `impl Fn(u64, Option<u64>) + Send + Sync + 'static` (not `FnMut` — use atomics for shared state; `total` is `None` when `Content-Length` is absent) and runs on the requesting task. `sha256`, `progress`, and `resume` are download-only and fail with `Kind::Config` on `send()`/`text()`/`bytes()`/`pages()`/`events()`/`websocket()`.
- **Files on disk.** Writes to `<name>.leyline.part` (plus `.part.json` metadata capped at 16 KiB and advisory `.leyline.lock`, mode `0600` on Unix) and renames atomically onto `path` only after success, leaving any existing target untouched on failure.

### One-shot download (inspect headers first)

When you want to inspect `Response` headers before deciding where or whether to
save (or want automatic decompression while saving), use `.stream()` +
`Response::download_to(path)` (one-shot; no resume or progress callbacks):

| Step | What happens |
| --- | --- |
| Status | With `RequestBuilder::error_for_status()`, `4xx`/`5xx` fails with `Kind::Status` (keeping up to 64 KiB of error body) and writes nothing; `Response::download_to` itself does not check status |
| Write | Streams decoded bytes to a temp `.<name>.<hex>.part` file in the same directory |
| Limit | Capped by `max_body_size` (100 MiB default) in decoded bytes |
| Commit | Flushes, syncs, and atomically renames onto `path` (preserving existing file permissions on Unix without setuid/setgid/sticky bits; once the blocking commit starts, it finishes even if the future is dropped) |
| Error or drop | Removes the temp `.part` file before commit starts, leaving `path` untouched |

A path with no file name fails with `Kind::Request`. If directory sync fails
after rename, `path` already holds the new file and the `Kind::Io` message says
so.

### Bound streamed bodies and downloads

For a `.stream()` request, `Timeout::total` covers only up to the response
headers (before the body arrives). Two timeouts bound the body itself:

- `read` limits the idle gap between chunks (catches a stalled server).
- `body` limits total body transfer time from the first read (catches a slow trickle).

See [Retries and timeouts](retries-and-timeouts.md#the-timeouts). To cancel a
download from your code (such as on Ctrl-C), drop its future; see
[Cancellation](cancellation.md).

## Stream a request body

For a file on disk, pass `Body::file("upload.bin")?` directly (no extra crate
needed; see [Requests](requests.md#query-parameters-and-bodies)). For any other
async source, `Body::stream(s, len)` takes a `Stream` of `io::Result<Bytes>` and
an `Option<u64>` length:

- Pass `Some(len)` whenever you know the length so Leyline sends `Content-Length` on every protocol (yielding more or fewer bytes than `len` fails the request).
- `Body::content_length()` returns the known byte length for buffered, file, and sized-stream bodies.
- Without a length, HTTP/1.1 uses `Transfer-Encoding: chunked` and HTTP/2 and HTTP/3 omit `Content-Length`.

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
    .send().await?;
println!("{}", resp.status());
# Ok(())
# }
```

When your stream returns an error, the request fails with `Kind::Body` (carrying
your error in `source()`). Leyline does not retry once bytes have been read:
HTTP/2 and HTTP/3 reset only that stream, while HTTP/1.1 closes the connection.

### When a body cannot be replayed

A streamed body is consumed by the first attempt, so:

- **Redirects:** `301`/`302`/`303` (rewritten to `GET`) follow normally; `307`/`308` (which must resend the body) fail with `Kind::Redirect`.
- **Retries:** Skipped once any byte has been read. Buffer the body as `Bytes` when retries or `307`/`308` redirects are needed.

## Stream a response body

Without `.stream()`, `send()` reads the entire body into memory (up to
`max_body_size`, 100 MiB default) before returning. Call `.stream()` so `send()`
returns as soon as the response headers arrive, then read chunks with
`bytes_stream()` and `Body::next().await` (no `futures-util` import needed):

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session
    .get("https://example.com/big")
    .max_body_size(64 * 1024 * 1024)
    .stream()
    .send()
    .await?;
let mut body = resp.bytes_stream()?;
let mut total = 0u64;
while let Some(chunk) = body.next().await {
    total += chunk?.len() as u64;
}
println!("{total} bytes");
# Ok(())
# }
```

`text()`, `bytes()`, and `json()` also work on a `.stream()` response (draining
and decoding it). A streamed response carries no trailers. `Body` also
implements `futures_util::Stream<Item = leyline::Result<Bytes>>`.

### Read the body as `AsyncRead`

`leyline::Error` converts into `std::io::Error` (keeping the `leyline::Error`
inside for `Error::find`), so a streamed `Body` plugs into
`tokio_util::io::StreamReader`:

```rust,no_run
use futures_util::TryStreamExt;
use tokio::io::AsyncReadExt;
use tokio_util::io::StreamReader;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session.get("https://example.com/big").stream().send().await?;
let body = resp.bytes_stream()?.map_err(std::io::Error::from);
let mut reader = std::pin::pin!(StreamReader::new(body));
let mut start = [0u8; 16];
match reader.read(&mut start).await {
    Ok(n) => println!("{:?}", &start[..n]),
    Err(err) => println!("{:?}", leyline::Error::find(&err).map(leyline::Error::kind)),
}
# Ok(())
# }
```

### Decoded or raw

`bytes_stream()`, `write_to(writer)`, and `Response::download_to(path)` decode
chunk by chunk and stop at `max_body_size` (failing with `Kind::BodyLimit`).
Pass `.decode(false)` to receive raw encoded bytes (useful in pass-through
proxies). See [Responses](responses.md#bodies).

## Stop at a marker

`Response::read_until(limit, done)` streams and decodes a `.stream()` response,
calling `done(body, from)` after each chunk (`body` holds all decoded bytes so
far; `from` is where the new chunk starts) and stopping as soon as `done`
returns `true` or `limit` decoded bytes arrive.

It returns `(bytes, matched)`: `matched` is `true` if your closure returned
`true`, and `false` whenever the response ended or hit `limit` bytes without a
match (compare `bytes.len() == limit` to tell those two apart; `limit == 0`
reads nothing).

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let marker = b"</head>";
let (head, matched) = session
    .get("https://example.com/big")
    .stream()
    .send()
    .await?
    .read_until(256 * 1024, |body, from| {
        let start = from.saturating_sub(marker.len() - 1);
        body[start..].windows(marker.len()).any(|w| w == marker)
    })
    .await?;
if matched {
    println!("head is {} bytes", head.len());
} else {
    println!("no </head> found in {} bytes", head.len());
}
# Ok(())
# }
```

Start each scan at `from.saturating_sub(marker.len() - 1)` so markers crossing
chunk boundaries match in linear time (`examples/stock_monitor.rs` uses this
pattern). Stopping early closes an HTTP/1.1 connection or resets the HTTP/2 /
HTTP/3 stream.

## Back-pressure

A streamed response body travels through a bounded channel so a slow reader
applies back-pressure instead of growing memory unboundedly:

| Protocol | Channel | When you stop reading |
| --- | --- | --- |
| HTTP/1.1 | 16 chunks | Socket reader pauses and TCP flow control slows the peer |
| HTTP/2 | 32 chunks | Driver serves other streams while withholding window updates for the stalled stream |
| HTTP/3 | 32 chunks | QUIC stream reads pause and QUIC flow control slows the peer |

Dropping `bytes_stream()` early cancels the transfer; see
[Cancellation](cancellation.md).

## Read server-sent events

`RequestBuilder::events()?` returns a lazy `leyline::sse::Events` stream
yielding `leyline::sse::Event` values. Await `next()` directly (no
`futures-util` needed) or use `futures_util::Stream`:

```rust,no_run
use leyline::{Retry, Session};

# async fn run() -> leyline::Result<()> {
let session = Session::new();
let mut events = session
    .get("https://api.example/events")
    .events()?
    .last_event_id("42")
    .retry(Retry::new(3))
    .max_event_size(1024 * 1024);
while let Some(event) = events.next().await {
    let event = event?;
    println!("{} {} {}", event.id(), event.event(), event.data());
}
# Ok(())
# }
```

- **Framing and parsing.** Requires `GET` without a body and a `200 text/event-stream` response. Handles UTF-8, leading BOM, multiline `data`, comments, and CR/LF/CRLF line endings. `event.json::<T>()` deserializes `data` with serde; incomplete trailing events are discarded.
- **Reconnection.** `Events::last_event_id(id)` seeds the initial `Last-Event-ID`; subsequent reconnects send the last id committed by a complete event block (an empty id clears it). `204` ends the stream. Defaults to 3 consecutive reconnects (resetting on each delivered event); `retry(0)` disables reconnects. Wait is `max(server retry, backoff)` capped by `max_wait`. Malformed responses and event-size errors are terminal.
- **Limits and cancellation.** `max_event_size` defaults to 1 MiB (pass `None` to remove the cap), and `max_body_size`, `read`, and `body` timeouts apply per connection. Dropping `Events` closes the active body or connect with no background reconnect task; cancelling a single `next()` future keeps the pending read in `Events`.
