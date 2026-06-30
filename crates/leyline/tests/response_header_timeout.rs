//! Regression gate for the time-to-first-byte (`response_header`) timeout.
//!
//! A proxy/upstream that completes the connection and then goes silent is the
//! classic hang: `connect_timeout` has already passed and `read_timeout` only
//! arms once body bytes flow, so before this knob the silent phase was bounded
//! only by the request-wide `total`. The test stands up a plaintext H1 server
//! that accepts the request and never replies, and asserts the client errors
//! with `Timeout` on the TTFB budget — well before `total`.

use std::time::{Duration, Instant};

use leyline::{Error, Session};
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

#[tokio::test]
async fn response_header_timeout_fires_when_upstream_goes_silent() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Server: accept, read the request bytes, then sit silent. Holding the
    // connection open (no response) is exactly the "connected but unresponsive
    // proxy" failure the TTFB cap exists to catch.
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
        // TTFB cap well under `total`: if the request errors near 300ms it was
        // the response-header timeout, not the 10s backstop.
        .response_header_timeout(Duration::from_millis(300))
        .timeout(Duration::from_secs(10))
        .build()
        .expect("session builds");

    let start = Instant::now();
    let result = session.get(&format!("http://{addr}/")).send().await;
    let elapsed = start.elapsed();

    assert!(
        matches!(result, Err(Error::Timeout)),
        "expected Error::Timeout, got {result:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "TTFB cap should fire near 300ms, took {elapsed:?} — likely fell through to `total`"
    );

    server.abort();
}

#[tokio::test]
async fn no_response_header_timeout_means_request_survives_past_the_ttfb_window() {
    // The knob is opt-in: with `response_header` unset, a brief pre-first-byte
    // delay must NOT error. Server stalls 600ms (longer than the prior test's
    // 300ms TTFB), then sends a normal response; the request succeeds.
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
        .send()
        .await
        .expect("request without a TTFB cap survives the 600ms stall");
    assert_eq!(resp.status(), 200);

    let _ = server.await;
}

#[tokio::test]
async fn session_recovers_after_ttfb_timeout_no_pool_wedge() {
    // Drop-safety at the session boundary: a TTFB timeout drops the in-flight
    // request future. The half-finished connection must NOT be returned to the
    // pool, and a subsequent request on the SAME session must succeed. If the
    // timed-out connection wedged the pool, the second request would reuse a
    // desynced socket and fail.
    //
    // Server accepts in a loop and handles each connection concurrently, so the
    // first connection's stall does not block answering the second.
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
                    // First connection: never reply — trip the TTFB cap.
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
        .response_header_timeout(Duration::from_millis(250))
        .timeout(Duration::from_secs(10))
        .build()
        .expect("session builds");

    let first = session.get(&format!("http://{addr}/")).send().await;
    assert!(
        matches!(first, Err(Error::Timeout)),
        "first request should TTFB-timeout, got {first:?}"
    );

    // Second request on the same session must recover and succeed.
    let second = session
        .get(&format!("http://{addr}/"))
        .send()
        .await
        .expect("session must recover and succeed after a TTFB timeout");
    assert_eq!(second.status(), 200);

    server.abort();
}

#[tokio::test]
async fn total_backstop_bounds_silence_when_ttfb_unset() {
    // Layering guard: with no TTFB cap, a silent upstream is STILL bounded — by
    // `total`, just later. Confirms TTFB is an early-out, not the only guard,
    // and that the two caps don't interfere when only `total` is set.
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
        // `total` only, no TTFB cap.
        .timeout(Duration::from_millis(400))
        .build()
        .expect("session builds");

    let start = Instant::now();
    let result = session.get(&format!("http://{addr}/")).send().await;
    let elapsed = start.elapsed();

    assert!(
        matches!(result, Err(Error::Timeout)),
        "expected total Timeout, got {result:?}"
    );
    assert!(
        elapsed >= Duration::from_millis(350) && elapsed < Duration::from_secs(2),
        "total backstop should fire near 400ms, took {elapsed:?}"
    );

    server.abort();
}
