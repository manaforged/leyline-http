//! Integration tests for the `RetryPolicy` — exercises the retry
//! loop against a mock H1 server that can return flaky responses.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use bytes::Bytes;
use futures_util::stream;
use leyline::core::{Body, RetryPolicy, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Read one complete HTTP/1.1 request off the socket and return when
/// headers are done. For the tests below, request bodies are either
/// absent (GET) or tiny enough to arrive with headers.
async fn read_one_request(sock: &mut tokio::net::TcpStream) {
    let mut buf = [0u8; 4096];
    let mut acc = Vec::new();
    loop {
        let n = sock.read(&mut buf).await.unwrap();
        if n == 0 {
            return;
        }
        acc.extend_from_slice(&buf[..n]);
        if acc.windows(4).any(|w| w == b"\r\n\r\n") {
            return;
        }
    }
}

#[tokio::test]
async fn retries_503_then_succeeds() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = counter.clone();

    let server = tokio::spawn(async move {
        for _ in 0..3 {
            let (mut sock, _) = listener.accept().await.unwrap();
            read_one_request(&mut sock).await;
            let n = counter_clone.fetch_add(1, Ordering::Relaxed);
            if n < 2 {
                sock.write_all(
                    b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            } else {
                sock.write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok",
                )
                .await
                .unwrap();
            }
            sock.flush().await.unwrap();
        }
    });

    let session = Session::builder().http1().build().unwrap();
    let policy =
        RetryPolicy::default().with_backoff(Duration::from_millis(1), Duration::from_millis(10));
    let resp = session
        .get(&format!("http://{addr}/flaky"))
        .retry(policy)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text(), "ok");
    assert_eq!(counter.load(Ordering::Relaxed), 3);
    server.await.unwrap();
}

#[tokio::test]
async fn does_not_retry_on_400() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = counter.clone();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        read_one_request(&mut sock).await;
        counter_clone.fetch_add(1, Ordering::Relaxed);
        sock.write_all(
            b"HTTP/1.1 400 Bad Request\r\ncontent-length: 3\r\nconnection: close\r\n\r\nbad",
        )
        .await
        .unwrap();
        sock.flush().await.unwrap();
    });

    let session = Session::builder().http1().build().unwrap();
    let policy =
        RetryPolicy::default().with_backoff(Duration::from_millis(1), Duration::from_millis(5));
    let resp = session
        .get(&format!("http://{addr}/bad"))
        .retry(policy)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    assert_eq!(counter.load(Ordering::Relaxed), 1);
    server.await.unwrap();
}

#[tokio::test]
async fn post_without_opt_in_does_not_retry() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = counter.clone();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        // Drain headers + body.
        let mut buf = [0u8; 4096];
        let mut acc = Vec::new();
        loop {
            let n = sock.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            acc.extend_from_slice(&buf[..n]);
            if acc.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        counter_clone.fetch_add(1, Ordering::Relaxed);
        sock.write_all(
            b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
        )
        .await
        .unwrap();
        sock.flush().await.unwrap();
    });

    let session = Session::builder().http1().build().unwrap();
    let policy =
        RetryPolicy::default().with_backoff(Duration::from_millis(1), Duration::from_millis(5));
    let resp = session
        .post(&format!("http://{addr}/payment"))
        .body(vec![1u8, 2, 3])
        .retry(policy)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 503);
    assert_eq!(counter.load(Ordering::Relaxed), 1);
    server.await.unwrap();
}

#[tokio::test]
async fn streaming_body_plus_retry_errors_clearly() {
    let session = Session::builder().http1().build().unwrap();
    // A streaming body that would have been replayable-hostile.
    let chunks: Vec<std::io::Result<Bytes>> = vec![Ok(Bytes::from_static(b"abc"))];
    let body = Body::stream(stream::iter(chunks));

    // Hitting an unreachable port produces a ConnectionError; with
    // retry on, the builder should recognise the stream body and fail
    // with a replay-specific message BEFORE attempting a second call.
    let policy = RetryPolicy::default()
        .with_max_retries(3)
        .with_backoff(Duration::from_millis(1), Duration::from_millis(5));
    let err = session
        .put("http://127.0.0.1:1/upload")
        .body(body)
        .retry(policy)
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("Streaming bodies cannot") || msg.contains("cannot be replayed"),
        "got: {msg}"
    );
}

#[tokio::test]
async fn retries_exhausted_returns_last_response() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = counter.clone();

    let server = tokio::spawn(async move {
        for _ in 0..4 {
            let (mut sock, _) = listener.accept().await.unwrap();
            read_one_request(&mut sock).await;
            counter_clone.fetch_add(1, Ordering::Relaxed);
            sock.write_all(
                b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
            )
            .await
            .unwrap();
            sock.flush().await.unwrap();
        }
    });

    let session = Session::builder().http1().build().unwrap();
    let policy = RetryPolicy::default()
        .with_max_retries(3)
        .with_backoff(Duration::from_millis(1), Duration::from_millis(5));
    let resp = session
        .get(&format!("http://{addr}/bad"))
        .retry(policy)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 502);
    // Original + 3 retries = 4 attempts.
    assert_eq!(counter.load(Ordering::Relaxed), 4);
    server.await.unwrap();
}
