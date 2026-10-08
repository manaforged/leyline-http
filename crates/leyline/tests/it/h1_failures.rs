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

struct Endless(Arc<std::sync::atomic::AtomicBool>);

impl futures_util::Stream for Endless {
    type Item = std::io::Result<Bytes>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::task::Poll::Ready(Some(Ok(Bytes::from(vec![b'u'; 64 * 1024]))))
    }
}

impl Drop for Endless {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn an_early_final_response_ends_an_upload_the_server_stopped_reading() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut head = Vec::new();
        let mut buf = [0u8; 1024];
        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
            let read = socket.read(&mut buf).await.unwrap();
            head.extend_from_slice(&buf[..read]);
        }
        socket
            .write_all(b"HTTP/1.1 413 Payload Too Large\r\ncontent-length: 0\r\n\r\n")
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        drop(socket);
    });
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    let sent = session
        .post(format!("http://{addr}/upload"))
        .body(Body::stream(Endless(Arc::clone(&dropped)), None))
        .send();
    let response = tokio::time::timeout(std::time::Duration::from_secs(3), sent)
        .await
        .expect("the early response arrives while the upload is blocked")
        .unwrap();
    assert_eq!(response.status(), 413);
    assert!(dropped.load(Ordering::SeqCst));
}

struct Watched<S> {
    inner: S,
    polled: Arc<AtomicUsize>,
    dropped: Arc<std::sync::atomic::AtomicBool>,
}

impl<S: futures_util::Stream + Unpin> futures_util::Stream for Watched<S> {
    type Item = S::Item;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.polled.fetch_add(1, Ordering::SeqCst);
        std::pin::Pin::new(&mut self.inner).poll_next(cx)
    }
}

impl<S> Drop for Watched<S> {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

async fn head_then(listener: TcpListener, reply: Option<&'static [u8]>) {
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        let read = socket.read(&mut buf).await.unwrap();
        head.extend_from_slice(&buf[..read]);
    }
    if let Some(reply) = reply {
        socket.write_all(reply).await.unwrap();
    }
    socket.shutdown().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    drop(socket);
}

fn watched<S>(
    inner: S,
) -> (
    Watched<S>,
    Arc<AtomicUsize>,
    Arc<std::sync::atomic::AtomicBool>,
) {
    let polled = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stream = Watched {
        inner,
        polled: Arc::clone(&polled),
        dropped: Arc::clone(&dropped),
    };
    (stream, polled, dropped)
}

#[tokio::test]
async fn a_peer_that_closes_before_answering_ends_a_waiting_upload() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(head_then(listener, None));
    let first = futures_util::stream::iter([Ok::<_, std::io::Error>(Bytes::from_static(b"part"))]);
    let (body, _, dropped) = watched(futures_util::StreamExt::chain(
        first,
        futures_util::stream::pending(),
    ));
    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    let sent = session
        .post(format!("http://{addr}/upload"))
        .body(Body::stream(body, None))
        .send();
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(3), sent)
        .await
        .expect("the closed response side ends the request");
    assert!(outcome.is_err(), "{outcome:?}");
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn ready_empty_chunks_do_not_hide_an_early_response() {
    const EMPTY_CHUNKS: usize = 50_000_000;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(head_then(
        listener,
        Some(b"HTTP/1.1 413 Payload Too Large\r\ncontent-length: 0\r\n\r\n"),
    ));
    let empties = futures_util::stream::iter(
        std::iter::repeat_with(|| Ok::<_, std::io::Error>(Bytes::new())).take(EMPTY_CHUNKS),
    );
    let (body, polled, dropped) = watched(empties);
    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    let response = session
        .post(format!("http://{addr}/upload"))
        .body(Body::stream(body, None))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 413);
    assert!(dropped.load(Ordering::SeqCst));
    assert!(
        polled.load(Ordering::SeqCst) < EMPTY_CHUNKS,
        "the upload ran every empty chunk"
    );
}
