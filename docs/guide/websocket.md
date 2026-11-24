# WebSocket

The `websocket` feature is on by default. A WebSocket uses the session's TLS
fingerprint, proxy rules, and cookie jar. Use `wss://`: any other scheme,
`ws://` included, fails with `Kind::Request` before Leyline picks a proxy or
connects.

## Connect, send, and receive

Build a request with `Session::get(url)` and call `websocket()` to open a
`WebSocket`. `send` takes a `Message`: `Text(String)`, `Binary(Vec<u8>)`,
`Ping(Vec<u8>)`, `Pong(Vec<u8>)`, or `Close(Option<(u16, String)>)`.

```rust,no_run
use leyline::Message;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let mut ws = session.get("wss://example.com/live").websocket().await?;

ws.send(Message::Text("ping".to_owned())).await?;
ws.send(Message::Binary(vec![1, 2, 3])).await?;

if let Some(msg) = ws.recv().await? {
    println!("{msg:?}");
}
ws.close().await?;
# Ok(())
# }
```

When the peer closes the connection, `recv` first returns
`Some(Message::Close(frame))` (`Option<(u16, String)>` with code and reason),
then `Ok(None)`. `close()` sends a close frame with no code and shuts the
connection down.

| Error `Kind` | When it happens |
| --- | --- |
| `Kind::Request` | Non-`wss://` scheme, bad handshake headers, non-101/non-200 upgrade, subprotocol mismatch, or ping/pong payload > 125 B |
| `Kind::Body` | Frame/message protocol violation, invalid UTF-8, or message/frame size limit exceeded |
| `Kind::Tls` | TLS handshake or post-handshake record alert |
| `Kind::Timeout` | Connect or handshake (`Timeout::total`) timeout |
| `Kind::Io` | Transport error or I/O on a closed connection |

## Inspect the handshake

| Method | Returns | Meaning |
| --- | --- | --- |
| `version()` | `http::Version` | Negotiated protocol (`HTTP_2` or `HTTP_11`) |
| `protocol()` | `Option<&str>` | Server-selected `Sec-WebSocket-Protocol`, if any |
| `headers()` | `&HeaderMap` | All handshake response headers |
| `header(name)` | `Option<&str>` | First visible-ASCII value for `name` (`None` if absent or non-ASCII) |

## Ping, pong, and close

`Message::Ping(payload)` and `Message::Pong(payload)` carry at most 125 bytes
(longer payloads fail `send` with `Kind::Request`). Leyline answers incoming
pings automatically and still returns them from `recv`. Send
`Message::Close(Some((code, reason)))` instead of `close()` to include a close
code and reason.

```rust,no_run
use leyline::Message;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let mut ws = session.get("wss://example.com/live").websocket().await?;

ws.send(Message::Ping(b"beat-7".to_vec())).await?;
match ws.recv().await? {
    Some(Message::Pong(payload)) => println!("pong {payload:?}"),
    other => println!("{other:?}"),
}
ws.send(Message::Close(Some((1000, "done".to_owned())))).await?;
# Ok(())
# }
```

## Set handshake and session options

Request-level setters on `RequestBuilder`:

| Builder method | Effect |
| --- | --- |
| `header(name, value)`, `headers(pairs)` | Append handshake headers; invalid values fail with `Kind::Request`. `user-agent` or `origin` replaces Leyline's value |
| `header("sec-websocket-protocol", "chat.v1, chat.v2")` | Offers subprotocols; fails with `Kind::Request` if the server picks an unoffered protocol, none, or more than one |
| `header_order(&[..])` | Pins regular handshake header wire order |
| `timeout(d)` | Bounds the handshake with `Timeout::total` |
| `protocol(Protocol)` | `Http1` skips H2; `Http2` requires extended `CONNECT` (fails with `Kind::Http2` without H1 fallback); `Http3` fails with `Kind::Config` |
| `proxy(url)` | Overrides the session proxy (`wss://` follows `https://` proxy and `NO_PROXY` rules) |

Session-level limits on `SessionBuilder` (for the 5 size/buffer setters,
passing `None` restores the library's default limit shown below, not
unlimited):

| Setter | Default | Effect |
| --- | --- | --- |
| `websocket_http2(bool)` | `true` | Try RFC 8441 HTTP/2 extended `CONNECT` before HTTP/1.1 upgrade |
| `websocket_max_message_size`, `websocket_max_frame_size` | 64 MiB / 16 MiB | Largest accepted message and frame |
| `websocket_read_buffer_size`, `websocket_write_buffer_size`, `websocket_max_write_buffer_size` | 128 KiB / 128 KiB / unlimited | Socket read/write buffer limits |
| `websocket_accept_unmasked_frames(bool)` | `false` | Accept unmasked frames from the peer |

The handshake sends matching cookies from the session jar (unless overridden by
a `Cookie` header) and stores any `Set-Cookie` headers from the handshake
response in the jar (unless `SessionBuilder::cookies(None)` was set).

## Shutdown and host limits

The handshake takes a host-limit slot for its origin (shared with `https://`
requests) and releases it once open. `Session::shutdown()` stops in-flight
handshakes and fails new ones with `err.is_shut_down() == true`; established
connections remain open until closed.

## HTTP/2 or HTTP/1.1

By default (`websocket_http2(true)`), Leyline multiplexes WebSockets onto a
pooled HTTP/2 connection (RFC 8441 extended `CONNECT`, matching modern Chrome
and Firefox) and falls back automatically to a classic HTTP/1.1 `101 Switching
Protocols` upgrade if the server does not negotiate `h2` or does not advertise
HTTP/2 WebSocket support (`SETTINGS_ENABLE_CONNECT_PROTOCOL`). Set
`websocket_http2(false)` if an intermediary or server mishandles HTTP/2
upgrades and you want to go straight to HTTP/1.1.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .websocket_max_message_size(1 << 20)
    .websocket_http2(false)
    .build()?;
let ws = session
    .get("wss://example.com/live")
    .header("sec-websocket-protocol", "chat.v1, chat.v2")
    .websocket()
    .await?;
println!("version={:?} protocol={:?}", ws.version(), ws.protocol());
# Ok(())
# }
```

## Browser-profile WebSockets

A WebSocket opened from a browser `Session` automatically reuses that session's
TLS fingerprint, proxy rules, cookies, and `User-Agent`, and sets `Origin` to
the WebSocket URL's `https://` origin. When the WebSocket endpoint lives on a
different subdomain or host than the page (for example `wss://ws.shop.example`
opened from `https://shop.example`), override `origin` explicitly:

```rust,no_run
use leyline::{Browser, Message, Session};

# async fn run() -> leyline::Result<()> {
let session = Session::from(Browser::Chrome158);
let mut ws = session
    .get("wss://ws.shop.example/live")
    .header("origin", "https://shop.example")
    .websocket()
    .await?;
ws.send(Message::Text("subscribe".to_owned())).await?;
# Ok(())
# }
```

Leyline does not negotiate `Sec-WebSocket-Extensions` (`permessage-deflate` is
not offered, and `sec-websocket-extensions` is reserved).

## Split the connection

`split()` returns `(impl Sink<Message> + Send + Unpin, impl Stream<Item = leyline::Result<Message>> + Send + Unpin)`
so one task can write while another reads. Closing the sink does not send a
close frame; send `Message::Close` instead.

```rust,no_run
use futures_util::{SinkExt, StreamExt};
use leyline::Message;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let ws = session.get("wss://example.com/live").websocket().await?;
let (mut sink, mut stream) = ws.split();

let reader = tokio::spawn(async move {
    while let Some(Ok(msg)) = stream.next().await {
        println!("{msg:?}");
    }
});

sink.send(Message::Text("hello".to_owned())).await?;
sink.send(Message::Close(None)).await?;
reader.await.expect("reader task panicked");
# Ok(())
# }
```
