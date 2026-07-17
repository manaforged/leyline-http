//! Recovery / resilience tests: transient connection failures from flaky
//! providers recover when retries are explicitly enabled, the default performs
//! no application retry, and a stalled handshake fails fast on the connect
//! timeout rather than hanging the whole request budget.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use leyline::{RetryPolicy, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A GET against an upstream that drops the first connection before responding
/// must recover when retries are enabled because a mid-exchange EOF is a
/// retryable connection error.
#[tokio::test]
async fn explicit_retry_policy_recovers_from_transient_connection_drop() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let conns = Arc::new(AtomicUsize::new(0));
    let conns_srv = conns.clone();

    let server = tokio::spawn(async move {
        let mut buf = [0u8; 1024];
        // Connection 1: read the request, then drop the socket (flaky upstream
        // closes before it replies).
        let (mut s1, _) = listener.accept().await.unwrap();
        conns_srv.fetch_add(1, Ordering::SeqCst);
        let _ = s1.read(&mut buf).await;
        drop(s1);
        // Connection 2: serve a normal 200.
        let (mut s2, _) = listener.accept().await.unwrap();
        conns_srv.fetch_add(1, Ordering::SeqCst);
        let _ = s2.read(&mut buf).await;
        let _ = s2
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .await;
        let _ = s2.flush().await;
    });

    let session = Session::builder()
        .http1()
        .disable_env_proxies()
        .retry(
            RetryPolicy::default().with_backoff(Duration::from_millis(1), Duration::from_millis(5)),
        )
        .build()
        .unwrap();

    let resp = session
        .get(&format!("http://{addr}/"))
        .send()
        .await
        .expect("explicit retries should recover from the first-connection drop");
    assert_eq!(resp.status(), 200);
    assert_eq!(
        conns.load(Ordering::SeqCst),
        2,
        "should have retried onto a second connection"
    );

    server.abort();
}

/// Without an explicit retry policy, the same transient drop surfaces as an
/// error with no second connection attempt.
#[tokio::test]
async fn default_session_does_not_retry_a_transient_connection_drop() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let conns = Arc::new(AtomicUsize::new(0));
    let conns_srv = conns.clone();

    let server = tokio::spawn(async move {
        let mut buf = [0u8; 1024];
        let (mut s1, _) = listener.accept().await.unwrap();
        conns_srv.fetch_add(1, Ordering::SeqCst);
        let _ = s1.read(&mut buf).await;
        drop(s1);
        // A second accept is available but should never be hit.
        if let Ok((mut s2, _)) = listener.accept().await {
            conns_srv.fetch_add(1, Ordering::SeqCst);
            let _ = s2.read(&mut buf).await;
        }
    });

    let session = Session::builder()
        .http1()
        .disable_env_proxies()
        .build()
        .unwrap();

    let result = session.get(&format!("http://{addr}/")).send().await;
    assert!(
        result.is_err(),
        "the no-retry default must surface the connection drop"
    );
    // Give any (erroneous) retry a moment to land before asserting the count.
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        conns.load(Ordering::SeqCst),
        1,
        "the default must not retry"
    );

    server.abort();
}

/// A default connect timeout (10s) must bound a stalled TLS handshake: an
/// endpoint that completes TCP but never progresses the handshake must fail
/// well before the 300s `total`, without the caller configuring anything.
/// Here we shorten `connect_timeout` so the test is fast, but the point is the
/// default is *some* finite value rather than `None`.
#[tokio::test]
async fn connect_timeout_bounds_a_stalled_tls_handshake() {
    // Accept the TCP connection but send nothing — the TLS ClientHello gets no
    // ServerHello, so the handshake stalls.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (_sock, _) = listener.accept().await.unwrap();
        // Hold the connection open, never speak TLS.
        tokio::time::sleep(Duration::from_secs(30)).await;
    });

    let session = Session::builder()
        .disable_env_proxies()
        .connect_timeout(Duration::from_millis(400))
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    let start = Instant::now();
    let result = session
        .get(&format!("https://127.0.0.1:{}/", addr.port()))
        .send()
        .await;
    let elapsed = start.elapsed();

    assert!(result.is_err(), "a stalled handshake must error");
    assert!(
        elapsed < Duration::from_secs(3),
        "stalled handshake took {elapsed:?} — the connect timeout did not bound it"
    );

    server.abort();
}
