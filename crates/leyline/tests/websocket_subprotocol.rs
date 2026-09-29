#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
#[path = "tls_support/mod.rs"]
mod tls_support;

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use leyline::http::header::SEC_WEBSOCKET_PROTOCOL;
use leyline::{Kind, ProtocolPolicy, Session, TlsTrustConfig, WsConnection};
use leyline_bssl::hash::{MessageDigest, hash};
use leyline_bssl_tokio::SslStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const OFFERED: &str = "chat, superchat";

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

async fn upgrade(mut stream: SslStream<TcpStream>, selected: &'static [&'static str]) {
    let Some(key) = read_key(&mut stream).await else {
        return;
    };
    let digest = hash(MessageDigest::sha1(), format!("{key}{GUID}").as_bytes()).unwrap();
    let mut reply = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n",
        STANDARD.encode(&*digest)
    );
    for protocol in selected {
        reply.push_str(&format!("Sec-WebSocket-Protocol: {protocol}\r\n"));
    }
    reply.push_str("\r\n");
    stream.write_all(reply.as_bytes()).await.unwrap();
    std::future::pending::<()>().await;
}

async fn connect(selected: &'static [&'static str]) -> leyline::Result<WsConnection> {
    let (cert, key) = tls_support::self_signed();
    let der = cert.to_der().unwrap();
    let port = tls_support::tls_server(cert, key, Arc::new(AtomicUsize::new(0)), move |stream| {
        upgrade(stream, selected)
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
        .header(SEC_WEBSOCKET_PROTOCOL, OFFERED)
        .connect()
        .await
}

#[tokio::test]
async fn an_offered_subprotocol_is_accepted() {
    let socket = connect(&["superchat"]).await.unwrap();
    assert_eq!(socket.protocol(), Some("superchat"));
}

#[tokio::test]
async fn a_subprotocol_the_client_did_not_offer_fails_the_handshake() {
    let err = connect(&["evil"]).await.unwrap_err();
    assert_eq!(err.kind(), Kind::Request, "{err:?}");
}

#[tokio::test]
async fn a_second_subprotocol_line_fails_the_handshake() {
    let err = connect(&["chat", "superchat"]).await.unwrap_err();
    assert_eq!(err.kind(), Kind::Request, "{err:?}");
}

#[tokio::test]
async fn an_offer_without_a_selected_subprotocol_fails_the_handshake() {
    let err = connect(&[]).await.unwrap_err();
    assert_eq!(err.kind(), Kind::Request, "{err:?}");
}
