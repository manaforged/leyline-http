# WebSocket

The `websocket` feature is on by default. A WebSocket uses the same session
for its TLS fingerprint, its proxy rules, and its cookie jar. The HTTP/1.1
upgrade opens a fresh connection instead of using the HTTP pool. Use `wss://`:
a `ws://` URL fails with `Kind::Request` before Leyline picks a proxy or
connects.

## Connect

`Session::websocket(url)` returns a `WebSocketBuilder`. Call `connect()`, or
await the builder.

```rust,no_run
use leyline::WsMessage;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let mut ws = session.websocket("wss://example.com/live").connect().await?;
ws.send(WsMessage::Text("hello".to_owned())).await?;
# Ok(())
# }
```

Builder methods:

- `config(WebSocketConfig)` sets limits for this connection.
- `header(name, value)` and `headers(pairs)` add handshake headers. A repeated
  name replaces the earlier value.
- `proxy(url)` overrides the session proxy.

## Cookies and proxies

The handshake sends the cookies that the session cookie jar holds for the URL,
matched as for an `https://` request. A `Cookie` header from the session
default headers or from `header()` replaces them. A `Set-Cookie` header in the
handshake response does not update the jar.

A `wss://` URL follows the same proxy rules as an `https://` URL: a rule for
`https` or for all schemes applies. Environment discovery and `NO_PROXY` work as
for any request. See [Proxies](proxies.md).

## HTTP/2 or HTTP/1.1

`WebSocketConfig::prefer_http2` defaults to `true`. Leyline first tries RFC
8441 extended `CONNECT` over HTTP/2, which reuses a pooled HTTP/2 connection.
Leyline logs the reason and falls back to the RFC 6455 HTTP/1.1 upgrade in two
cases:

- The TLS handshake does not select `h2`, for example because the origin
  selects `http/1.1`. Leyline then remembers that the origin speaks HTTP/1.1
  only, and later WebSockets to it skip the HTTP/2 attempt.
- The peer does not advertise `SETTINGS_ENABLE_CONNECT_PROTOCOL`.

Any other error is returned rather than retried.

`WebSocketConfig::new().prefer_http2(false)` skips the HTTP/2 attempt.
`protocol()` returns the subprotocol the origin selected, if any.

```rust,no_run
use leyline::WebSocketConfig;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let ws = session
    .websocket("wss://example.com/live")
    .config(
        WebSocketConfig::new()
            .max_message_size(1 << 20)
            .prefer_http2(false),
    )
    .connect()
    .await?;
println!("protocol={:?}", ws.protocol());
# Ok(())
# }
```

## Send and receive

`send` takes a `WsMessage`, Leyline's own message enum (`Text`, `Binary`,
`Ping`, `Pong`, `Close`). When the peer closes the connection, `recv` first
returns `Some(WsMessage::Close(frame))`, where `frame` is an
`Option<CloseFrame>` with the close `code` and `reason`. The next `recv`
returns `Ok(None)`. `close` sends a close frame and shuts the connection down.

```rust,no_run
use leyline::WsMessage;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let mut ws = session.websocket("wss://example.com/live").connect().await?;

ws.send(WsMessage::Text("ping".to_owned())).await?;
ws.send(WsMessage::Binary(vec![1, 2, 3])).await?;

if let Some(msg) = ws.recv().await? {
    println!("{msg:?}");
}
ws.close().await?;
# Ok(())
# }
```

## Split the connection

`split()` divides the connection into a `WsSink` and a `WsStream`, so one task
can write while another reads. The sink keeps `send` and `close`. The stream keeps `recv`.

```rust,no_run
use leyline::WsMessage;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let ws = session.websocket("wss://example.com/live").connect().await?;
let (mut sink, mut stream) = ws.split();

let reader = tokio::spawn(async move {
    while let Ok(Some(msg)) = stream.recv().await {
        println!("{msg:?}");
    }
});

sink.send(WsMessage::Text("hello".to_owned())).await?;
sink.close().await?;
reader.await.expect("reader task panicked");
# Ok(())
# }
```

## Next

Read [HTTP/3](http3.md).
