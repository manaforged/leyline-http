# WebSocket

The `websocket` feature is on by default. A WebSocket uses the same session
for its TLS fingerprint and proxy. It does not read or write the session
cookie jar, and the HTTP/1.1 upgrade opens a fresh connection instead of
using the HTTP pool. Send cookies as explicit handshake headers when the
origin needs them, and use `wss://`: plaintext `ws://` is not accepted.

## Connect

`Session::websocket(url)` returns a `WebSocketBuilder`. Call `connect()`, or
await the builder.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::chrome();
let mut ws = session.websocket("wss://example.com/live").connect().await?;
ws.send("hello").await?;
# Ok(())
# }
```

Builder methods:

- `config(WebSocketConfig)` sets limits for this connection.
- `header(name, value)` and `headers(pairs)` add handshake headers. A repeated
  name replaces the earlier value.
- `proxy(url)` overrides the session proxy.
- `http1()` forces the HTTP/1.1 path.

## HTTP/2 or HTTP/1.1

`WebSocketConfig::prefer_http2` defaults to `true`. Leyline first tries RFC
8441 extended `CONNECT` over HTTP/2, which reuses a pooled HTTP/2 connection.
If the peer does not advertise `SETTINGS_ENABLE_CONNECT_PROTOCOL`, Leyline
logs the reason and falls back to the RFC 6455 HTTP/1.1 upgrade. Any other
error is returned rather than retried.

`http1()` on the builder, or `prefer_http2: false` in the config, skips the
HTTP/2 attempt. `WsConnection::is_http2()` tells you which transport you got,
and `protocol()` returns the subprotocol the origin selected, if any.

```rust,no_run
use leyline::WebSocketConfig;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::chrome();
let ws = session
    .websocket("wss://example.com/live")
    .config(WebSocketConfig::default().max_message_size(1 << 20))
    .http1()
    .connect()
    .await?;
println!("http2={} protocol={:?}", ws.is_http2(), ws.protocol());
# Ok(())
# }
```

## Send and receive

`send` takes text, `send_binary` takes bytes, and `send_raw` takes a
`WsMessage`, Leyline's own message enum (`Text`, `Binary`, `Ping`, `Pong`,
`Close`). `recv` returns
`Ok(None)` when the peer closed. `close` sends a close frame and shuts the
connection down.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::chrome();
let mut ws = session.websocket("wss://example.com/live").connect().await?;

ws.send("ping").await?;
ws.send_binary(vec![1, 2, 3]).await?;

if let Some(msg) = ws.recv().await? {
    println!("{msg:?}");
}
ws.close().await?;
# Ok(())
# }
```

## Split the connection

`split()` divides the connection into a `WsSink` and a `WsStream`, so one task
can write while another reads. The sink keeps `send`, `send_binary`,
`send_raw`, and `close`. The stream keeps `recv`.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::chrome();
let ws = session.websocket("wss://example.com/live").connect().await?;
let (mut sink, mut stream) = ws.split();

let reader = tokio::spawn(async move {
    while let Ok(Some(msg)) = stream.recv().await {
        println!("{msg:?}");
    }
});

sink.send("hello").await?;
sink.close().await?;
reader.await.expect("reader task panicked");
# Ok(())
# }
```

## Next

Read [HTTP/3](http3.md).
