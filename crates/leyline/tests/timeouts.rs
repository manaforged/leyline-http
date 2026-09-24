use std::time::{Duration, Instant};

use futures_util::StreamExt;
use leyline::http::Method;
use leyline::{ProtocolPolicy, ProxyConfig, RetryPolicy, Session, TimeoutConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn serve<F, Fut>(after: F) -> std::net::SocketAddr
where
    F: FnOnce(tokio::net::TcpStream) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    drop(tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.expect("accept");
        let mut buf = [0u8; 2048];
        drop(sock.read(&mut buf).await);
        after(sock).await;
    }));
    addr
}

#[tokio::test]
async fn connect_timeout_fires_on_a_black_hole_address() {
    let session = Session::builder()
        .proxy(ProxyConfig::new().without_env())
        .timeout(
            TimeoutConfig::new()
                .total(Duration::from_secs(10))
                .connect(Duration::from_millis(300)),
        )
        .build()
        .expect("session builds");

    let start = Instant::now();
    let err = session
        .request(Method::GET, "https://192.0.2.1:81/")
        .retry(RetryPolicy::none())
        .send()
        .await
        .expect_err("a black-hole address must not connect");
    let elapsed = start.elapsed();

    assert!(err.is_timeout(), "expected a timeout, got {err:?}");
    assert!(
        elapsed < Duration::from_secs(2),
        "connect cap should fire near 300ms, took {elapsed:?}"
    );
}

#[tokio::test]
async fn response_header_timeout_fires_when_the_server_never_writes() {
    let addr = serve(|sock| async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        drop(sock);
    })
    .await;

    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .proxy(ProxyConfig::new().without_env())
        .build()
        .expect("session builds");

    let start = Instant::now();
    let err = session
        .request(Method::GET, format!("http://{addr}/"))
        .timeout(
            TimeoutConfig::default()
                .total(Duration::from_secs(10))
                .response_header(Duration::from_millis(300)),
        )
        .send()
        .await
        .expect_err("a silent server must not resolve");
    let elapsed = start.elapsed();

    assert!(err.is_timeout(), "expected a timeout, got {err:?}");
    assert!(
        elapsed < Duration::from_secs(2),
        "per-request TTFB cap should fire near 300ms, took {elapsed:?}"
    );
}

async fn half_chunked(mut sock: tokio::net::TcpStream) {
    drop(
        sock.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nhalf\r\n")
            .await,
    );
    drop(sock.flush().await);
    tokio::time::sleep(Duration::from_secs(30)).await;
    drop(sock);
}

#[tokio::test]
async fn read_timeout_fires_between_chunks_of_a_streamed_body() {
    let addr = serve(half_chunked).await;

    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .proxy(ProxyConfig::new().without_env())
        .build()
        .expect("session builds");

    let resp = session
        .request(Method::GET, format!("http://{addr}/"))
        .timeout(
            TimeoutConfig::default()
                .total(Duration::from_secs(10))
                .read(Duration::from_millis(300)),
        )
        .stream()
        .send()
        .await
        .expect("headers arrive before the stall");
    assert_eq!(resp.status(), 200);

    let start = Instant::now();
    let mut body = resp.into_stream().expect("streaming body");
    let first = body
        .next()
        .await
        .expect("first chunk")
        .expect("chunk is ok");
    assert_eq!(&first[..], b"half");
    let stalled = body.next().await.expect("a stalled stream yields an error");
    let elapsed = start.elapsed();

    let err = leyline::Error::from(stalled.expect_err("second chunk must time out"));
    assert!(err.is_timeout(), "expected a timeout, got {err:?}");
    assert!(
        elapsed < Duration::from_secs(2),
        "read cap should fire near 300ms, took {elapsed:?}"
    );
}

#[tokio::test]
async fn read_timeout_does_not_cover_a_buffered_body() {
    let addr = serve(half_chunked).await;

    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .proxy(ProxyConfig::new().without_env())
        .build()
        .expect("session builds");

    let start = Instant::now();
    let err = session
        .request(Method::GET, format!("http://{addr}/"))
        .timeout(
            TimeoutConfig::default()
                .total(Duration::from_secs(10))
                .read(Duration::from_millis(200))
                .response_header(Duration::from_millis(700)),
        )
        .send()
        .await
        .expect_err("a half-written buffered body must not resolve");
    let elapsed = start.elapsed();

    assert!(err.is_timeout(), "expected a timeout, got {err:?}");
    assert!(
        elapsed >= Duration::from_millis(600),
        "a buffered body is read inside the TTFB window, so `read` must not clip it at 200ms; bailed at {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "TTFB cap should fire near 700ms, took {elapsed:?}"
    );
}

#[tokio::test]
async fn total_timeout_bounds_a_slow_server() {
    let addr = serve(|mut sock| async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        drop(
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .await,
        );
    })
    .await;

    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .proxy(ProxyConfig::new().without_env())
        .build()
        .expect("session builds");

    let start = Instant::now();
    let err = session
        .request(Method::GET, format!("http://{addr}/"))
        .timeout(TimeoutConfig::default().total(Duration::from_millis(400)))
        .send()
        .await
        .expect_err("a 30s server must not beat a 400ms total");
    let elapsed = start.elapsed();

    assert!(err.is_timeout(), "expected a timeout, got {err:?}");
    assert!(
        elapsed >= Duration::from_millis(350) && elapsed < Duration::from_secs(2),
        "total cap should fire near 400ms, took {elapsed:?}"
    );
}
