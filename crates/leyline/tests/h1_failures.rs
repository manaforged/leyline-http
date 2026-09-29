#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bytes::Bytes;
use leyline::{Body, Kind, ProtocolPolicy, ProxyConfig, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const OK: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
const TWO_LENGTHS: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 3\r\n\r\nok";

#[derive(Default)]
struct Counts {
    connections: AtomicUsize,
    requests: AtomicUsize,
}

async fn read_head(socket: &mut TcpStream) -> bool {
    let mut seen = Vec::new();
    let mut buf = [0u8; 1024];
    while !seen.windows(4).any(|window| window == b"\r\n\r\n") {
        match socket.read(&mut buf).await {
            Ok(0) | Err(_) => return false,
            Ok(n) => seen.extend_from_slice(&buf[..n]),
        }
    }
    true
}

async fn answer(mut socket: TcpStream, replies: Arc<[Option<&'static [u8]>]>, counts: Arc<Counts>) {
    while read_head(&mut socket).await {
        let index = counts.requests.fetch_add(1, Ordering::SeqCst);
        let Some(reply) = replies[index.min(replies.len() - 1)] else {
            return;
        };
        if socket.write_all(reply).await.is_err() {
            return;
        }
    }
}

async fn server(replies: &[Option<&'static [u8]>]) -> (SocketAddr, Arc<Counts>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let counts = Arc::new(Counts::default());
    let replies: Arc<[Option<&'static [u8]>]> = replies.into();
    let shared = counts.clone();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            shared.connections.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(answer(socket, replies.clone(), shared.clone()));
        }
    });
    (addr, counts)
}

fn session() -> Session {
    Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .proxy(ProxyConfig::new().env(false))
        .build()
        .unwrap()
}

#[tokio::test]
async fn a_response_that_breaks_the_wire_format_is_an_invalid_data_io_error() {
    let (addr, _) = server(&[Some(TWO_LENGTHS)]).await;
    let err = session().get(format!("http://{addr}/")).await.unwrap_err();
    assert_eq!(err.kind(), Kind::Io, "{err:?}");
    assert_eq!(
        err.io().map(std::io::Error::kind),
        Some(std::io::ErrorKind::InvalidData),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_malformed_response_on_a_reused_connection_is_not_sent_again() {
    let (addr, counts) = server(&[Some(OK), Some(TWO_LENGTHS)]).await;
    let session = session();
    let url = format!("http://{addr}/");
    session.get(&url).await.unwrap().bytes().await.unwrap();
    session.get(&url).await.unwrap_err();
    assert_eq!(counts.requests.load(Ordering::SeqCst), 2);
    assert_eq!(counts.connections.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_streaming_body_on_a_dead_pooled_connection_is_a_body_error() {
    let (addr, counts) = server(&[Some(OK), None]).await;
    let session = session();
    let url = format!("http://{addr}/");
    session.get(&url).await.unwrap().bytes().await.unwrap();
    let chunks =
        futures_util::stream::iter([Ok::<_, std::io::Error>(Bytes::from_static(b"payload"))]);
    let err = session
        .put(&url)
        .body(Body::stream(chunks, None))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Body, "{err:?}");
    assert_eq!(counts.connections.load(Ordering::SeqCst), 1);
}
