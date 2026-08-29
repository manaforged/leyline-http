//! Regression gate for the `connect_timeout` phase boundary.
//!
//! `connect_timeout` bounds DNS + TCP + (proxy `CONNECT`) + TLS — connection
//! *establishment* only. The contract that matters for a non-idempotent write
//! like a place-order POST: once the connection is up, `connect_timeout` is spent
//! and must NEVER bound the in-flight request/response. A timeout knob that bled
//! into the response phase would clip a slow-but-live order, so these tests pin
//! the boundary: a short `connect_timeout` with a slow-but-alive server still
//! succeeds, while a dead endpoint bails on the connect phase instead of hanging
//! to the request-wide `total`.
use std::time::{Duration, Instant};

use leyline::{RetryPolicy, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn connect_timeout_does_not_bound_the_response_phase() {
    // Server accepts immediately (so connect succeeds well inside the 300ms
    // connect budget), then takes 600ms of "think time" before replying —
    // longer than connect_timeout. If connect_timeout wrongly governed the
    // response phase, this would error at ~300ms instead of returning 200.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        let _ = sock.read(&mut buf).await;
        tokio::time::sleep(Duration::from_millis(600)).await;
        let _ = sock
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .await;
        let _ = sock.flush().await;
    });

    let session = Session::builder()
        .http1()
        .disable_env_proxies()
        .connect_timeout(Duration::from_millis(300))
        .timeout(Duration::from_secs(10))
        .build()
        .expect("session builds");

    let start = Instant::now();
    let resp = session
        .get(&format!("http://{addr}/"))
        .await
        .expect("a 300ms connect timeout must not clip a response served at 600ms");
    let elapsed = start.elapsed();

    assert_eq!(resp.status(), 200);
    assert!(
        elapsed >= Duration::from_millis(550),
        "returned in {elapsed:?}, before the server's 600ms think time — the response was short-circuited, not served"
    );

    let _ = server.await;
}

#[tokio::test]
async fn connect_timeout_bounds_an_unreachable_endpoint() {
    // 192.0.2.1 is TEST-NET-1 (RFC 5737): reserved and unroutable, so the TCP
    // connect never completes. With a 300ms connect timeout and a 10s `total`,
    // the attempt must bail on the connect phase (well under `total`), proving a
    // dead proxy/origin fails fast instead of hanging the whole request budget.
    //
    // HTTPS + the default Auto policy on purpose: that is the path real traffic
    // uses, and `connect_timeout` is enforced there (via the TLS connector).
    // Note: the plaintext `http://` path connects via a bare `TcpStream::connect`
    // that ignores `connect_timeout`, and the explicit `.http1()` HTTPS path does
    // not honor it either — both are bounded only by `total`. Auto/h2 is the
    // path under test here.
    //
    // Retry is disabled to isolate establishment. A single logical request still
    // makes a small number of connect attempts internally, so the bound is a few
    // multiples of `connect_timeout` — the assertion is "fail-fast, well under
    // `total`", not an exact multiple.
    let session = Session::builder()
        .disable_env_proxies()
        .connect_timeout(Duration::from_millis(300))
        .timeout(Duration::from_secs(10))
        .build()
        .expect("session builds");

    let start = Instant::now();
    let result = session
        .request("GET", "https://192.0.2.1:443/")
        .retry(RetryPolicy::none())
        .send()
        .await;
    let elapsed = start.elapsed();

    assert!(
        result.is_err(),
        "unreachable endpoint must error, got {result:?}"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "connect bail took {elapsed:?} — connect_timeout did not bound it (fell through toward the 10s total)"
    );
}
