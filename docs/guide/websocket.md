# WebSocket

The `websocket` feature is on by default. A WebSocket uses the session's TLS
fingerprint, proxy rules, and cookie jar. Use `wss://`: any other scheme,
`ws://` included, fails with `Kind::Request` before Leyline picks a proxy or
connects.

## Connect, send, and receive

`Session::websocket(url)` returns a `WebSocketBuilder`. Call `connect()`, or
await the builder. `send` takes a `WsMessage`: `Text`, `Binary`, `Ping`,
`Pong`, or `Close`.

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

When the peer closes the connection, `recv` first returns
`Some(WsMessage::Close(frame))`, where `frame` is an `Option<CloseFrame>`
with the close `code` and `reason`. The next `recv` returns `Ok(None)`.
`close` sends a close frame and shuts the connection down.

A `send`, `recv`, or `close` that fails on the transport or on a closed
connection gives `Kind::Io`. A protocol violation, or a message over the
`WebSocketConfig` limits, gives `Kind::Body`.

## Set handshake options

| Builder method | Effect |
| --- | --- |
| `config(WebSocketConfig)` | Limits and transport for this connection |
| `header(name, value)`, `headers(pairs)` | Append handshake headers. A repeated name adds a value. An invalid name or value fails `connect()` with `Kind::Request`. A `user-agent` or `origin` header replaces the one Leyline sends |
| `proxy(url)` | Overrides the session proxy |

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

`WebSocketConfig::prefer_http2` defaults to `true`: Leyline first tries RFC
8441 extended `CONNECT` on a pooled HTTP/2 connection. It falls back to the
RFC 6455 HTTP/1.1 upgrade on a fresh connection in two cases, and logs the
reason:

- The TLS handshake does not select `h2`. Later WebSockets to that origin
  skip the HTTP/2 attempt.
- The peer does not advertise `SETTINGS_ENABLE_CONNECT_PROTOCOL`.

Any other error is returned, not retried. `prefer_http2(false)` skips the
HTTP/2 attempt. `protocol()` returns the subprotocol the origin selected.

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

## Split the connection

`split()` returns a `WsSink` with `send` and `close`, and a `WsStream` with
`recv`, so one task can write while another reads.

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
