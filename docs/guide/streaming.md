# Streaming

Stream when a body is larger than you want in memory, or when you want to act
on the first bytes before the last ones arrive. This chapter covers resumable
file downloads, streaming request and response bodies, `read_until`, and
server-sent events (SSE).

Streaming needs no extra Leyline feature. Some examples on this page use
other crates:

| Call or type | Extra crate |
| --- | --- |
| `Body::next()`, `Body::file()`, `Events::next()` | None |
| `tokio::fs::File` | `tokio` with feature `fs` |
| `ReaderStream`, `StreamReader` | `tokio-util` with feature `io` |
| `TryStreamExt`, `StreamExt` combinators | `futures-util` |

`leyline::Bytes` re-exports `bytes::Bytes` for custom `Body::stream` sources.

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

With `.retry(n)`, a dropped transfer resumes from the last saved checkpoint.
Calling `download_to(path)` again later resumes the same checkpoint.
`Timeout::total` still spans every retry, so clear or raise it for a long
download.

| Topic | Behavior |
| --- | --- |
| Request | `GET` only, with no body and no caller `Range` or `If-Range` header. Otherwise `download_to` fails with `Kind::Config` |
| Encoding | Asks for `identity` (uncompressed) bytes so byte offsets stay exact |
| Resume | Sends `Range` and `If-Range` when the server sent a strong `ETag`, such as `"abc123"` and not `W/"abc123"`. The final URL, ETag, byte range, and total length must also match. Otherwise the download restarts on a `200` |
| Checkpoints | Synced to disk after at least 1 MiB and 1 second since the last one, and on a body error |
| Checksum | `.sha256([u8; 32])` checks the complete file, resumed prefix included, before the rename |
| Fresh start | `.resume(false)` ignores a saved partial file |
| Progress | `.progress` takes `impl Fn(u64, Option<u64>) + Send + Sync + 'static`, so use atomics for shared state. `total` is `None` without `Content-Length`. It runs on the requesting task |
| Files | Writes `<name>.leyline.part`, `<name>.leyline.part.json` (metadata, at most 16 KiB), and an advisory `<name>.leyline.lock`, mode `0600` on Unix. It renames onto `path` only after success. On failure the existing target is untouched |

`sha256`, `progress`, and `resume` apply only to `download_to`. On `send()`,
`text()`, `bytes()`, `pages()`, `events()`, or `websocket()`, they fail with
`Kind::Config`.

### One-shot download (inspect headers first)

To read the `Response` headers before you decide whether or where to save,
send with `.stream()` and call `Response::download_to(path)`. This call also
decodes a compressed body. It saves in one pass, with no resume and no
progress callback:

| Step | What happens |
| --- | --- |
| Status | `Response::download_to` does not check the status. Add `RequestBuilder::error_for_status()` to fail a `4xx` or `5xx` with `Kind::Status` before anything is written; see [Errors](errors.md#status-codes-are-not-errors) |
| Write | Streams decoded bytes to a temporary `.<name>.<hex>.part` file in the same directory |
| Limit | Capped by `max_body_size` in decoded bytes |
| Commit | Flushes, syncs, and renames the file onto `path` in one atomic step. On Unix it keeps the existing file's permissions, without setuid, setgid, or sticky bits |
| Error | Removes the temporary file and leaves `path` untouched. [Cancellation](cancellation.md#drop-a-request-before-send-returns) covers a dropped future |

A path with no file name fails with `Kind::Request`. If directory sync fails
after rename, `path` already holds the new file and the `Kind::Io` message says
so.

### Bound streamed bodies and downloads

For a `.stream()` request, `Timeout::total` ends at the response headers.
Bound the body with the `read` and `body` timeouts; see
[Retries and timeouts](retries-and-timeouts.md#the-timeouts). To cancel a
download from your code, such as on Ctrl-C, drop its future; see
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

The first attempt consumes a streamed body, so Leyline cannot send it again:

- A redirect that rewrites the request to `GET` follows. Every other redirect fails with `Kind::Redirect`; [Redirects](redirects.md#what-a-redirect-changes) lists which statuses rewrite.
- A retry is skipped once any byte of the body was read.

Buffer the body as `Bytes` when you need retries or redirects that keep the
body.

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
and decoding it). `Body` also
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
pattern). [Cancellation](cancellation.md#drop-a-streamed-response) covers
what stopping early does on the wire.

## Back-pressure

A streamed response body travels through a bounded channel so a slow reader
applies back-pressure instead of growing memory unboundedly:

| Protocol | Channel | When you stop reading |
| --- | --- | --- |
| HTTP/1.1 | 32 chunks | Socket reader pauses and TCP flow control slows the peer |
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

Parsing:

- `events()` needs a `GET` without a body. The response must be a `200` with `text/event-stream`.
- The parser handles UTF-8, a leading BOM, multiline `data`, comments, and CR, LF, or CRLF line endings.
- `event.json::<T>()` deserializes `data` with serde. An incomplete event at the end of the stream is dropped.

Reconnection:

- `Events::last_event_id(id)` sets the first `Last-Event-ID`. Each reconnect sends the id of the last complete event. An empty id clears it.
- A `204` response ends the stream.
- By default, `Events` reconnects up to 3 times in a row. The count resets on each delivered event. `retry(0)` turns reconnects off.
- The wait is the larger of the server's `retry:` value and the backoff. Without a `retry:` value, the wait is the larger of the backoff and 3 s (or `max_wait`, if shorter).
- A wait longer than `max_wait` ends the stream with the error. So do a malformed response and an event over the size limit.

Limits and cancellation:

- `max_event_size` defaults to 1 MiB. Pass `None` to remove the cap.
- `max_body_size` and the `read` and `body` timeouts apply to each connection.
- Dropping `Events` closes the open body or connect. No reconnect task keeps running.
- Cancelling one `next()` future keeps the pending read in `Events`.
