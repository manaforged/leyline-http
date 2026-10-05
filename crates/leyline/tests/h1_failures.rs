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
const TRUNCATED: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc";

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
        if socket.write_all(reply).await.is_err() || reply == TRUNCATED {
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

fn failing_body() -> Body {
    let chunks =
        futures_util::stream::iter([Err::<Bytes, _>(std::io::Error::other("caller body failed"))]);
    Body::stream(chunks, None)
}

#[tokio::test]
async fn a_failing_request_body_stream_is_a_body_error() {
    let (addr, _) = server(&[Some(OK)]).await;
    let err = session()
        .put(format!("http://{addr}/"))
        .body(failing_body())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Body, "{err:?}");
    assert_eq!(
        err.io().map(ToString::to_string).as_deref(),
        Some("caller body failed"),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_failing_request_body_stream_is_not_a_dead_connection() {
    let (addr, _) = server(&[Some(OK)]).await;
    let session = session();
    let url = format!("http://{addr}/");
    session.get(&url).await.unwrap().bytes().await.unwrap();
    let err = session.put(&url).body(failing_body()).await.unwrap_err();
    assert_eq!(err.kind(), Kind::Body, "{err:?}");
    assert_eq!(session.pool_stats().evictions_dead, 0);
}

#[tokio::test]
async fn error_for_status_resends_a_success_cut_short_on_a_stale_pooled_connection() {
    let (addr, counts) = server(&[Some(OK), Some(TRUNCATED), Some(OK)]).await;
    let session = session();
    let url = format!("http://{addr}/");
    session
        .get(&url)
        .error_for_status()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let body = session
        .get(&url)
        .error_for_status()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(&body[..], b"ok");
    assert_eq!(counts.connections.load(Ordering::SeqCst), 2);
}

#[derive(Clone, Default)]
struct Reuse(Arc<std::sync::Mutex<Vec<bool>>>);

impl leyline::trace::Trace for Reuse {
    fn connect(&self, event: &leyline::trace::Connect<'_>) {
        self.0.lock().unwrap().push(event.reused);
    }
}

#[tokio::test]
async fn error_for_status_traces_a_reused_pooled_connection() {
    let (addr, _) = server(&[Some(OK)]).await;
    let reuse = Reuse::default();
    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .proxy(ProxyConfig::new().env(false))
        .trace(reuse.clone())
        .build()
        .unwrap();
    let url = format!("http://{addr}/");
    for _ in 0..2 {
        session
            .get(&url)
            .error_for_status()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
    }
    assert_eq!(reuse.0.lock().unwrap().as_slice(), &[false, true]);
}
