use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use http::StatusCode;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tower_layer::Layer;
use tower_service::Service;

use super::{Call, Pending, Reply};
use crate::core::error::{Error, Result};
use crate::{Browser, Session};

type Log = Arc<Mutex<Vec<String>>>;

/// Serve `/one` as a redirect to `/two` and every other path as `200 ok`, recording each request head.
async fn serve(log: Log) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let log = Arc::clone(&log);
            tokio::spawn(async move {
                let mut buf: Vec<u8> = Vec::new();
                let mut tmp = [0u8; 1024];
                loop {
                    let Ok(n) = socket.read(&mut tmp).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    while let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..end]).into_owned();
                        drop(buf.drain(..end + 4));
                        let reply: &[u8] = if head.starts_with("GET /one ") {
                            b"HTTP/1.1 302 Found\r\nlocation: /two\r\ncontent-length: 0\r\n\r\n"
                        } else {
                            b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok"
                        };
                        log.lock().unwrap().push(head);
                        if socket.write_all(reply).await.is_err() {
                            return;
                        }
                    }
                }
            });
        }
    });
    addr
}

/// Layer that counts every call it sees.
#[derive(Clone)]
struct Count(Arc<AtomicUsize>);

impl<S> Layer<S> for Count {
    type Service = Counter<S>;

    fn layer(&self, inner: S) -> Counter<S> {
        Counter {
            inner,
            hits: Arc::clone(&self.0),
        }
    }
}

/// The service [`Count`] installs.
#[derive(Clone)]
struct Counter<S> {
    inner: S,
    hits: Arc<AtomicUsize>,
}

impl<S> Service<Call> for Counter<S>
where
    S: Service<Call, Response = Reply, Error = Error>,
    S::Future: Send + 'static,
{
    type Response = Reply;
    type Error = Error;
    type Future = Pending;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<()>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, call: Call) -> Pending {
        self.hits.fetch_add(1, Ordering::SeqCst);
        Box::pin(self.inner.call(call))
    }
}

/// Layer that answers without touching the transport.
#[derive(Clone)]
struct Canned;

impl<S> Layer<S> for Canned {
    type Service = Short<S>;

    fn layer(&self, inner: S) -> Short<S> {
        Short { inner }
    }
}

/// The service [`Canned`] installs.
#[derive(Clone)]
struct Short<S> {
    #[expect(
        dead_code,
        reason = "a short-circuit layer never calls the inner service"
    )]
    inner: S,
}

impl<S> Service<Call> for Short<S>
where
    S: Service<Call, Response = Reply, Error = Error>,
{
    type Response = Reply;
    type Error = Error;
    type Future = Pending;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _call: Call) -> Pending {
        Box::pin(async {
            Ok(Reply::new(StatusCode::OK)
                .header("x-canned", "1")?
                .body("hi"))
        })
    }
}

/// Layer that adds one request header.
#[derive(Clone)]
struct Tag;

impl<S> Layer<S> for Tag {
    type Service = Tagged<S>;

    fn layer(&self, inner: S) -> Tagged<S> {
        Tagged { inner }
    }
}

/// The service [`Tag`] installs.
#[derive(Clone)]
struct Tagged<S> {
    inner: S,
}

impl<S> Service<Call> for Tagged<S>
where
    S: Service<Call, Response = Reply, Error = Error>,
    S::Future: Send + 'static,
{
    type Response = Reply;
    type Error = Error;
    type Future = Pending;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<()>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut call: Call) -> Pending {
        call.headers_mut().append("x-layer", "yes").unwrap();
        Box::pin(self.inner.call(call))
    }
}

#[tokio::test]
async fn one_call_per_redirect_leg() {
    let log: Log = Arc::default();
    let addr = serve(Arc::clone(&log)).await;
    let hits = Arc::new(AtomicUsize::new(0));

    let session = Session::builder()
        .browser(Browser::Chrome147)
        .layer(Count(Arc::clone(&hits)))
        .build()
        .unwrap();
    let mut resp = session.get(&format!("http://{addr}/one")).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "ok");
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(log.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn short_circuit_skips_the_transport() {
    let session = Session::builder()
        .browser(Browser::Chrome147)
        .layer(Canned)
        .build()
        .unwrap();
    let mut resp = session.get("http://127.0.0.1:1/nothing").await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.header("x-canned"), Some("1"));
    assert_eq!(resp.text().await.unwrap(), "hi");
}

#[tokio::test]
async fn added_header_reaches_the_wire() {
    let log: Log = Arc::default();
    let addr = serve(Arc::clone(&log)).await;

    let session = Session::builder()
        .browser(Browser::Chrome147)
        .layer(Tag)
        .build()
        .unwrap();
    let resp = session.get(&format!("http://{addr}/two")).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let seen = log.lock().unwrap().join("\n");
    assert!(seen.contains("x-layer: yes"), "{seen}");
}

#[tokio::test]
async fn no_layer_keeps_the_direct_path() {
    let log: Log = Arc::default();
    let addr = serve(Arc::clone(&log)).await;

    let mut resp = Session::chrome()
        .get(&format!("http://{addr}/one"))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "ok");
    let seen = log.lock().unwrap().join("\n");
    assert!(!seen.contains("x-layer"), "{seen}");
}
