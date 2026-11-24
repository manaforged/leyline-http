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
`Some(Message::Close(frame))`, where `frame` is an `Option<(u16, String)>`
with the close code and reason. The next `recv` returns `Ok(None)`.
`close` sends a close frame with no code and shuts the connection down. See
[Ping, pong, and close](#ping-pong-and-close) to send a code and a reason.

A `send`, `recv`, or `close` that fails on the transport or on a closed
connection gives `Kind::Io`. A protocol violation, or a message over the
session's WebSocket limits, gives `Kind::Body`.

## Ping, pong, and close

`Message::Ping(payload)` and `Message::Pong(payload)` carry a payload of at
most 125 bytes; pass `Vec::new()` for an empty one. A longer payload fails
`send` with `Kind::Request`. Leyline answers a ping from the peer for you,
and `recv` also returns it.

To close with a status code and a reason, send
`Message::Close(Some((code, reason)))` in place of `close()`.

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

## Set handshake options

The request builder methods set the handshake:

| Builder method | Effect |
| --- | --- |
| `header(name, value)`, `headers(pairs)` | Append handshake headers. A repeated name adds a value. An invalid name or value fails `websocket()` with `Kind::Request`. A `user-agent` or `origin` header replaces the one Leyline sends |
| `header("sec-websocket-protocol", "chat.v1, chat.v2")` | Offers subprotocols. `WebSocket::protocol()` returns the one the server selected, or `None`. The handshake fails when the server selects one you did not offer |
| `proxy(url)` | Overrides the session proxy |

Limits and transport are set on the session, with these `SessionBuilder`
setters:

| Setter | Effect |
| --- | --- |
| `websocket_http2(bool)` | Try HTTP/2 first; see below |
| `websocket_max_message_size`, `websocket_max_frame_size` | Largest message and frame accepted |
| `websocket_read_buffer_size`, `websocket_write_buffer_size`, `websocket_max_write_buffer_size` | Buffer sizes |
| `websocket_accept_unmasked_frames(bool)` | Accept unmasked frames from the peer |

The handshake sends the cookies that the jar holds for the URL, matched as
for an `https://` request. A `Cookie` header from the session default headers
or from `header()` replaces them. A `Set-Cookie` header in the handshake
response does not update the jar.

A `wss://` URL follows the proxy rules of an `https://` URL, including
environment discovery and `NO_PROXY`. See [Proxies](proxies.md).

## Shutdown and host limits

The handshake takes a host-limit slot for its origin, the same slot an
`https://` request to that host takes, and releases it when the handshake
completes or fails. An open connection holds no slot. `Session::shutdown()`
stops a handshake in flight and fails new ones with the shut-down error. It
does not close an established connection.

## HTTP/2 or HTTP/1.1

`websocket_http2` defaults to `true`: Leyline first tries RFC
8441 extended `CONNECT` on a pooled HTTP/2 connection. It falls back to the
RFC 6455 HTTP/1.1 upgrade on a fresh connection in two cases, and logs the
reason:

- The TLS handshake does not select `h2`. Later WebSockets to that origin
  skip the HTTP/2 attempt.
- The peer does not advertise `SETTINGS_ENABLE_CONNECT_PROTOCOL`.

Any other error is returned, not retried. `websocket_http2(false)` skips the
HTTP/2 attempt.

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
println!("protocol={:?}", ws.protocol());
# Ok(())
# }
```

## Split the connection

`split()` returns a `futures_util::Sink<Message>` and a
`futures_util::Stream<Item = leyline::Result<Message>>`, so one task can write
while another reads. Pin each half before you use it. Closing the sink does
not send a close frame; send `Message::Close` instead.

```rust,no_run
use futures_util::{SinkExt, StreamExt};
use leyline::Message;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let ws = session.get("wss://example.com/live").websocket().await?;
let (sink, stream) = ws.split();
let mut sink = Box::pin(sink);
let mut stream = Box::pin(stream);

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

## Next

Read [HTTP/3](http3.md).
