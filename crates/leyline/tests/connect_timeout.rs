//! Regression gate for the `connect_timeout` phase boundary.
use std::time::{Duration, Instant};

use leyline::{RetryPolicy, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn connect_timeout_does_not_bound_the_response_phase() {
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
