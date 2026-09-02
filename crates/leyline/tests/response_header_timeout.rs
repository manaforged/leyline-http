//! Regression gate for the time-to-first-byte (`response_header`) timeout.
use std::time::{Duration, Instant};

use leyline::{Error, Session};
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

#[tokio::test]
async fn response_header_timeout_fires_when_upstream_goes_silent() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        let _ = sock.read(&mut buf).await;
        tokio::time::sleep(Duration::from_secs(30)).await;
        drop(sock);
    });

    let session = Session::builder()
        .http1()
        .disable_env_proxies()
        .timeouts(
            leyline::TimeoutConfig::default()
                .total(Duration::from_secs(10))
                .response_header(Duration::from_millis(300)),
        )
        .build()
        .expect("session builds");

    let start = Instant::now();
    let result = session.get(&format!("http://{addr}/")).await;
    let elapsed = start.elapsed();

    assert!(
        result.as_ref().is_err_and(Error::is_timeout),
        "expected Error::new(Kind::Timeout), got {result:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "TTFB cap should fire near 300ms, took {elapsed:?} — likely fell through to `total`"
    );

    server.abort();
}

#[tokio::test]
async fn no_response_header_timeout_means_request_survives_past_the_ttfb_window() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        let _ = sock.read(&mut buf).await;
        tokio::time::sleep(Duration::from_millis(600)).await;
        use tokio::io::AsyncWriteExt;
        let _ = sock
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .await;
        let _ = sock.flush().await;
    });

    let session = Session::builder()
        .http1()
        .disable_env_proxies()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("session builds");

    let resp = session
        .get(&format!("http://{addr}/"))
        .await
        .expect("request without a TTFB cap survives the 600ms stall");
    assert_eq!(resp.status(), 200);

    let _ = server.await;
}

#[tokio::test]
async fn session_recovers_after_ttfb_timeout_no_pool_wedge() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let mut n = 0u32;
        loop {
            let (mut sock, _) = listener.accept().await.unwrap();
            n += 1;
            let stall = n == 1;
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                if stall {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                } else {
                    use tokio::io::AsyncWriteExt;
                    let _ = sock
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                        .await;
                    let _ = sock.flush().await;
                }
            });
        }
    });

    let session = Session::builder()
        .http1()
        .disable_env_proxies()
        .timeouts(
            leyline::TimeoutConfig::default()
                .total(Duration::from_secs(10))
                .response_header(Duration::from_millis(250)),
        )
        .build()
        .expect("session builds");

    let first = session.get(&format!("http://{addr}/")).await;
    assert!(
        first.as_ref().is_err_and(Error::is_timeout),
        "first request should TTFB-timeout, got {first:?}"
    );

    let second = session
        .get(&format!("http://{addr}/"))
        .await
        .expect("session must recover and succeed after a TTFB timeout");
    assert_eq!(second.status(), 200);

    server.abort();
}

#[tokio::test]
async fn total_backstop_bounds_silence_when_ttfb_unset() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        let _ = sock.read(&mut buf).await;
        tokio::time::sleep(Duration::from_secs(30)).await;
        drop(sock);
    });

    let session = Session::builder()
        .http1()
        .disable_env_proxies()
        .timeout(Duration::from_millis(400))
        .build()
        .expect("session builds");

    let start = Instant::now();
    let result = session.get(&format!("http://{addr}/")).await;
    let elapsed = start.elapsed();

    assert!(
        result.as_ref().is_err_and(Error::is_timeout),
        "expected total Timeout, got {result:?}"
    );
    assert!(
        elapsed >= Duration::from_millis(350) && elapsed < Duration::from_secs(2),
        "total backstop should fire near 400ms, took {elapsed:?}"
    );

    server.abort();
}
