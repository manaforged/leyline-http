use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use leyline::testing::{TestResponse, TestServer, queue};
use leyline::{CompressionConfig, Session, TimeoutConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn a_slow_error_body_stops_at_the_error_body_timeout() {
    let server = TestServer::http(queue([TestResponse::new(500).chunks(
        std::iter::repeat_n("x".repeat(64), 40),
        Duration::from_millis(100),
    )]))
    .await
    .unwrap();
    let session = Session::builder()
        .timeout(
            TimeoutConfig::new()
                .total(Duration::from_secs(30))
                .error_body(Duration::from_millis(200)),
        )
        .build()
        .unwrap();

    let started = Instant::now();
    let err = session
        .get(server.url("/"))
        .error_for_status()
        .await
        .unwrap_err();

    assert_eq!(err.status().map(|s| s.as_u16()), Some(500));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn an_error_body_larger_than_the_body_limit_is_a_status_error() {
    let server = TestServer::http(queue([TestResponse::new(500).body(vec![b'x'; 2 << 20])]))
        .await
        .unwrap();
    let session = Session::builder()
        .compression(CompressionConfig::new().max_body_size(1 << 20))
        .build()
        .unwrap();

    let err = session
        .get(server.url("/"))
        .error_for_status()
        .await
        .unwrap_err();

    assert_eq!(err.status().map(|s| s.as_u16()), Some(500), "{err}");
    assert_eq!(err.body().map(<[u8]>::len), Some(64 * 1024));
}

async fn keep_alive_server(accepted: Arc<AtomicUsize>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            accepted.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 1024];
                loop {
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        match stream.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                    let head = String::from_utf8_lossy(&buf[..end]).into_owned();
                    buf.drain(..end);
                    let reply: &[u8] = if head.starts_with("GET /start ") {
                        b"HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 5\r\n\r\nmoved"
                    } else {
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok"
                    };
                    if stream.write_all(reply).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    port
}

#[tokio::test]
async fn error_for_status_reuses_the_connection_across_a_redirect() {
    let accepted = Arc::new(AtomicUsize::new(0));
    let port = keep_alive_server(Arc::clone(&accepted)).await;
    let session = Session::builder().build().unwrap();

    let response = session
        .get(format!("http://127.0.0.1:{port}/start"))
        .error_for_status()
        .await
        .unwrap();

    assert_eq!(response.text().await.unwrap(), "ok");
    assert_eq!(accepted.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn error_for_status_keeps_the_response_header_timeout_on_a_slow_success_body() {
    let server = TestServer::http(queue([
        TestResponse::new(200).chunks(std::iter::repeat_n("x", 10), Duration::from_millis(300))
    ]))
    .await
    .unwrap();
    let session = Session::builder()
        .timeout(
            TimeoutConfig::new()
                .total(Duration::from_secs(30))
                .response_header(Duration::from_millis(500)),
        )
        .build()
        .unwrap();

    let started = Instant::now();
    let err = session
        .get(server.url("/"))
        .error_for_status()
        .await
        .unwrap_err();

    assert!(err.is_timeout(), "{err}");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}
