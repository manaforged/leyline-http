#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
#[path = "tls_support/mod.rs"]
mod tls_support;

use std::time::Duration;

use leyline::testing::{TestResponse, TestServer};
use leyline::{HostLimits, Session};

#[tokio::test]
async fn a_failed_body_stream_that_is_kept_releases_its_connection_and_host_slot() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut head = Vec::new();
        let mut buf = [0u8; 1024];
        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
            let read = socket.read(&mut buf).await.unwrap();
            head.extend_from_slice(&buf[..read]);
        }
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\ncontent-length: 100000\r\n\r\n",
            )
            .await
            .unwrap();
        let closed = matches!(socket.read(&mut buf).await, Ok(0) | Err(_));
        closed_tx.send(closed).unwrap();
    });
    let session = Session::builder()
        .host_limits(HostLimits::new().max_in_flight(1))
        .build()
        .unwrap();
    let mut body = session
        .get(format!("http://{addr}/"))
        .timeout(leyline::TimeoutConfig::new().body(Duration::from_millis(200)))
        .stream()
        .await
        .unwrap()
        .into_stream()
        .unwrap();
    assert!(
        futures_util::StreamExt::next(&mut body)
            .await
            .unwrap()
            .is_err()
    );
    let closed = tokio::time::timeout(Duration::from_secs(2), closed_rx).await;
    assert!(
        matches!(closed, Ok(Ok(true))),
        "the connection stayed open behind a failed stream: {closed:?}"
    );
    assert!(session.host_stats().iter().all(|s| s.in_flight() == 0));
    drop(body);
}

#[tokio::test]
async fn a_shut_down_session_opens_no_websocket_and_no_preconnect() {
    let server = TestServer::https(|_| TestResponse::new(200)).await.unwrap();
    let session = Session::builder()
        .tls_trust(server.trust())
        .build()
        .unwrap();
    session.shutdown();
    let port = server.addr().port();
    let ws = session
        .websocket(format!("wss://localhost:{port}/feed"))
        .connect()
        .await
        .err()
        .unwrap();
    assert!(ws.is_shut_down(), "{ws:?}");
    let warm = session
        .preconnect(format!("https://localhost:{port}/"))
        .await
        .unwrap_err();
    assert!(warm.is_shut_down(), "{warm:?}");
    let plain = session
        .preconnect(format!("http://localhost:{port}/"))
        .await
        .unwrap_err();
    assert!(plain.is_shut_down(), "{plain:?}");
    assert!(server.requests().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_ends_a_websocket_handshake_in_flight() {
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use tokio::io::AsyncReadExt;
    let (cert, key) = tls_support::self_signed();
    let der = cert.to_der().unwrap();
    let (seen_tx, seen_rx) = tokio::sync::mpsc::unbounded_channel();
    let port = tls_support::tls_server_per_connection(
        cert,
        key,
        Arc::new(AtomicUsize::new(0)),
        |_| tls_support::HTTP11,
        move |_, mut stream| {
            let seen_tx = seen_tx.clone();
            async move {
                let mut head = [0u8; 1024];
                if let Ok(read) = stream.read(&mut head).await {
                    seen_tx.send(read).unwrap();
                }
                tokio::time::sleep(Duration::from_secs(30)).await;
                drop(stream);
            }
        },
    )
    .await;
    let session = Session::builder()
        .tls_trust(
            leyline::TlsTrustConfig::new()
                .env_roots(false)
                .system_roots(false)
                .add_ca_der(der),
        )
        .build()
        .unwrap();
    let handshake = tokio::spawn({
        let session = session.clone();
        async move {
            session
                .websocket(format!("wss://127.0.0.1:{port}/feed"))
                .connect()
                .await
        }
    });
    let mut seen_rx = seen_rx;
    let seen = tokio::time::timeout(Duration::from_secs(5), seen_rx.recv()).await;
    assert!(matches!(seen, Ok(Some(read)) if read > 0), "{seen:?}");
    session.shutdown();
    let err = tokio::time::timeout(Duration::from_secs(2), handshake)
        .await
        .expect("shutdown ends the stalled handshake")
        .unwrap()
        .err()
        .unwrap();
    assert!(err.is_shut_down(), "{err:?}");
}
