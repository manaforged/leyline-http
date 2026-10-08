use std::time::{Duration, Instant};

use leyline::{ProtocolPolicy, ProxyConfig, RetryPolicy, Session, TimeoutConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn connect_timeout_does_not_bound_the_response_phase() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        drop(sock.read(&mut buf).await);
        tokio::time::sleep(Duration::from_millis(600)).await;
        drop(
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .await,
        );
        drop(sock.flush().await);
    });

    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .proxy(ProxyConfig::new().env(false))
        .timeout(
            TimeoutConfig::new()
                .total(Duration::from_secs(10))
                .connect(Duration::from_millis(300)),
        )
        .build()
        .expect("session builds");

    let start = Instant::now();
    let resp = session
        .get(format!("http://{addr}/"))
        .await
        .expect("a 300ms connect timeout must not clip a response served at 600ms");
    let elapsed = start.elapsed();

    assert_eq!(resp.status(), 200);
    assert!(
        elapsed >= Duration::from_millis(550),
        "returned in {elapsed:?}, before the server's 600ms think time — the response was short-circuited, not served"
    );

    drop(server.await);
}

#[tokio::test]
async fn connect_timeout_bounds_an_unreachable_endpoint() {
    let session = Session::builder()
        .proxy(ProxyConfig::new().env(false))
        .timeout(
            TimeoutConfig::new()
                .total(Duration::from_secs(10))
                .connect(Duration::from_millis(300)),
        )
        .build()
        .expect("session builds");

    let start = Instant::now();
    let result = session
        .request(http::Method::GET, "https://192.0.2.1:443/")
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

#[tokio::test]
async fn connect_timeout_is_one_window_per_request_on_http_and_https() {
    let session = Session::builder()
        .proxy(ProxyConfig::new().env(false))
        .timeout(TimeoutConfig::new().connect(Duration::from_millis(300)))
        .build()
        .expect("session builds");

    for url in ["http://192.0.2.1:80/", "https://192.0.2.1:443/"] {
        let start = Instant::now();
        let result = session
            .request(http::Method::GET, url)
            .retry(RetryPolicy::none())
            .send()
            .await;
        let elapsed = start.elapsed();

        assert!(result.is_err(), "{url} must error, got {result:?}");
        assert!(
            elapsed < Duration::from_millis(550),
            "{url} took {elapsed:?}; the 300ms connect timeout must bound the whole request"
        );
    }
}
