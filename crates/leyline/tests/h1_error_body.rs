#[path = "core_support/forward.rs"]
mod forward;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use forward::counting_forwarder;
use leyline::testing::{TestResponse, TestServer, queue};
use leyline::{CompressionConfig, ProtocolPolicy, Session, TimeoutConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const STALLED_500: &[u8] = b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 2\r\n\r\nx";
const OK_200: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";

async fn read_head(socket: &mut TcpStream) -> bool {
    let mut seen = Vec::new();
    let mut buf = [0u8; 1024];
    while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
        match socket.read(&mut buf).await {
            Ok(0) | Err(_) => return false,
            Ok(n) => seen.extend_from_slice(&buf[..n]),
        }
    }
    true
}

async fn stalling_server(replies: &'static [&'static [u8]]) -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepted = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&accepted);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let index = count.fetch_add(1, Ordering::SeqCst);
            let reply = replies[index.min(replies.len() - 1)];
            tokio::spawn(async move {
                while read_head(&mut socket).await {
                    if socket.write_all(reply).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    (port, accepted)
}

fn session(error_body: Duration) -> Session {
    Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .timeout(
            TimeoutConfig::new()
                .total(Duration::from_secs(20))
                .response_header(Duration::from_secs(20))
                .body(Duration::from_secs(20))
                .error_body(error_body),
        )
        .build()
        .unwrap()
}

#[tokio::test]
async fn a_stalled_small_error_body_returns_its_prefix_at_the_error_body_timeout() {
    let (port, _) = stalling_server(&[STALLED_500]).await;

    let started = Instant::now();
    let err = session(Duration::from_millis(300))
        .get(format!("http://127.0.0.1:{port}/"))
        .error_for_status()
        .await
        .unwrap_err();

    assert_eq!(err.status().map(|s| s.as_u16()), Some(500), "{err}");
    assert_eq!(err.body(), Some(&b"x"[..]));
    assert_eq!(err.header("content-length"), None);
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_millis(250), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
}

#[tokio::test]
async fn a_stalled_error_body_does_not_return_its_connection_to_the_pool() {
    let (port, accepted) = stalling_server(&[STALLED_500, OK_200]).await;
    let session = session(Duration::from_millis(200));
    let url = format!("http://127.0.0.1:{port}/");

    session.get(&url).error_for_status().await.unwrap_err();
    let body = session.get(&url).await.unwrap().text().await.unwrap();

    assert_eq!(body, "ok");
    assert_eq!(accepted.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_completed_small_error_body_keeps_its_connection() {
    let server = TestServer::http(queue([
        TestResponse::new(500).body("boom"),
        TestResponse::new(200).body("ok"),
    ]))
    .await
    .unwrap();
    let (port, accepted) = counting_forwarder(server.addr()).await;
    let session = session(Duration::from_secs(5));
    let url = format!("http://127.0.0.1:{port}/");

    let err = session.get(&url).error_for_status().await.unwrap_err();
    let body = session.get(&url).await.unwrap().text().await.unwrap();

    assert_eq!(err.body(), Some(&b"boom"[..]));
    assert_eq!(body, "ok");
    assert_eq!(accepted.load(Ordering::SeqCst), 1);
}

async fn paced_error_server(total: usize, first: usize) -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepted = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&accepted);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let fresh = count.fetch_add(1, Ordering::SeqCst) == 0;
            tokio::spawn(async move {
                if !read_head(&mut socket).await {
                    return;
                }
                if !fresh {
                    drop(socket.write_all(OK_200).await);
                    return;
                }
                let head = format!(
                    "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {total}\r\n\r\n"
                );
                drop(socket.write_all(head.as_bytes()).await);
                drop(socket.write_all(&vec![b'e'; first]).await);
                tokio::time::sleep(Duration::from_millis(300)).await;
                drop(socket.write_all(&vec![b'e'; total - first]).await);
                if read_head(&mut socket).await {
                    drop(socket.write_all(OK_200).await);
                }
            });
        }
    });
    (port, accepted)
}

#[tokio::test]
async fn a_small_max_error_body_does_not_read_a_larger_error_body() {
    let (port, accepted) = paced_error_server(60 * 1024, 1024).await;
    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .compression(CompressionConfig::new().max_error_body(4))
        .build()
        .unwrap();
    let url = format!("http://127.0.0.1:{port}/");

    let started = Instant::now();
    let err = session.get(&url).error_for_status().await.unwrap_err();
    let elapsed = started.elapsed();
    let body = session.get(&url).await.unwrap().text().await.unwrap();

    assert_eq!(err.body(), Some(&b"eeee"[..]));
    assert!(elapsed < Duration::from_millis(250), "{elapsed:?}");
    assert_eq!(body, "ok");
    assert_eq!(accepted.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn an_error_body_that_is_not_decoded_keeps_its_content_encoding() {
    let server = TestServer::http(queue([
        TestResponse::new(500)
            .header("content-encoding", "gzip")
            .body(b"\x1f\x8b raw gzip bytes".to_vec()),
        TestResponse::new(500)
            .header("content-encoding", "x-custom")
            .body("opaque"),
    ]))
    .await
    .unwrap();
    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .compression(CompressionConfig::none())
        .build()
        .unwrap();

    let gzip = session
        .get(server.url("/"))
        .error_for_status()
        .await
        .unwrap_err();
    let custom = session
        .get(server.url("/"))
        .error_for_status()
        .await
        .unwrap_err();

    assert_eq!(gzip.header("content-encoding"), Some("gzip"));
    assert_eq!(gzip.body(), Some(&b"\x1f\x8b raw gzip bytes"[..]));
    assert_eq!(custom.header("content-encoding"), Some("x-custom"));
    assert_eq!(custom.body(), Some(&b"opaque"[..]));
}

#[tokio::test]
async fn a_smaller_max_body_size_bounds_the_error_read() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_head(&mut socket).await;
        drop(
            socket
                .write_all(
                    b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 61440\r\n\r\neeee",
                )
                .await,
        );
        tokio::time::sleep(Duration::from_secs(10)).await;
    });
    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .compression(CompressionConfig::new().max_body_size(4))
        .timeout(TimeoutConfig::new().error_body(Duration::from_millis(500)))
        .build()
        .unwrap();

    let started = Instant::now();
    let err = session
        .get(format!("http://127.0.0.1:{port}/"))
        .error_for_status()
        .await
        .unwrap_err();

    assert_eq!(err.status().map(|s| s.as_u16()), Some(500), "{err}");
    assert_eq!(err.body(), Some(&b"eeee"[..]));
    assert!(
        started.elapsed() < Duration::from_millis(250),
        "{:?}",
        started.elapsed()
    );
}
