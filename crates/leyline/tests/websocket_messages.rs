#![cfg(feature = "websocket")]
#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
#[path = "tls_support/mod.rs"]
mod tls_support;

use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures_util::{SinkExt, StreamExt};
use leyline::{CloseFrame, ProtocolPolicy, Session, TlsTrustConfig, WsConnection, WsMessage};
use leyline_bssl::hash::{MessageDigest, hash};
use leyline_bssl_tokio::SslStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::Role;

const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

type Seen = Arc<Mutex<Vec<Message>>>;

async fn read_key(stream: &mut SslStream<TcpStream>) -> Option<String> {
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut buf).await.ok()?;
        if read == 0 {
            return None;
        }
        head.extend_from_slice(&buf[..read]);
    }
    String::from_utf8_lossy(&head).lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("sec-websocket-key")
            .then(|| value.trim().to_owned())
    })
}

async fn echo(mut stream: SslStream<TcpStream>, seen: Seen) {
    let Some(key) = read_key(&mut stream).await else {
        return;
    };
    let digest = hash(MessageDigest::sha1(), format!("{key}{GUID}").as_bytes()).unwrap();
    let reply = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
        STANDARD.encode(&*digest)
    );
    stream.write_all(reply.as_bytes()).await.unwrap();
    let mut socket = WebSocketStream::from_raw_socket(stream, Role::Server, None).await;
    while let Some(Ok(message)) = socket.next().await {
        seen.lock().unwrap().push(message.clone());
        match message {
            Message::Text(_) | Message::Binary(_) => socket.send(message).await.unwrap(),
            Message::Close(_) => break,
            _ => {}
        }
    }
}

async fn connect(seen: Seen) -> WsConnection {
    let (cert, key) = tls_support::self_signed();
    let der = cert.to_der().unwrap();
    let port = tls_support::tls_server(cert, key, Arc::new(AtomicUsize::new(0)), move |stream| {
        echo(stream, Arc::clone(&seen))
    })
    .await;
    Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .tls_trust(
            TlsTrustConfig::new()
                .env_roots(false)
                .system_roots(false)
                .add_ca_der(der),
        )
        .build()
        .unwrap()
        .websocket(format!("wss://127.0.0.1:{port}/"))
        .connect()
        .await
        .unwrap()
}

#[tokio::test]
async fn text_and_binary_frames_round_trip_and_close_reaches_the_server() {
    let seen = Seen::default();
    let mut socket = connect(Arc::clone(&seen)).await;

    socket.send(WsMessage::Text("hello".into())).await.unwrap();
    socket
        .send(WsMessage::Binary(vec![0, 1, 254, 255]))
        .await
        .unwrap();
    assert_eq!(
        socket.recv().await.unwrap(),
        Some(WsMessage::Text("hello".into()))
    );
    assert_eq!(
        socket.recv().await.unwrap(),
        Some(WsMessage::Binary(vec![0, 1, 254, 255]))
    );
    socket.close().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen[0], Message::Text("hello".into()));
    assert_eq!(seen[1], Message::Binary(vec![0, 1, 254, 255].into()));
    assert!(matches!(seen[2], Message::Close(_)), "{seen:?}");
}

#[tokio::test]
async fn a_split_socket_sends_on_the_sink_and_reads_on_the_stream() {
    let seen = Seen::default();
    let (mut sink, mut stream) = connect(Arc::clone(&seen)).await.split();

    sink.send(WsMessage::Text("split".into())).await.unwrap();
    assert_eq!(
        stream.recv().await.unwrap(),
        Some(WsMessage::Text("split".into()))
    );
    sink.close().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen[0], Message::Text("split".into()));
    assert!(matches!(seen.last(), Some(Message::Close(_))), "{seen:?}");
}

#[tokio::test]
async fn a_close_frame_carries_its_code_and_reason_to_the_server() {
    let seen = Seen::default();
    let mut socket = connect(Arc::clone(&seen)).await;

    socket
        .send(WsMessage::Close(Some(CloseFrame::new(4001, "done"))))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let seen = seen.lock().unwrap().clone();
    let Some(Message::Close(Some(frame))) = seen.last() else {
        panic!("no close frame: {seen:?}");
    };
    assert_eq!(u16::from(frame.code), 4001);
    assert_eq!(frame.reason.as_str(), "done");
}
