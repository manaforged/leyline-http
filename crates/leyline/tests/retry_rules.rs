use std::io::Write;
use std::time::Duration;

use bytes::Bytes;
use leyline::testing::{TestResponse, TestServer, queue};
use leyline::{Body, CompressionConfig, RetryPolicy, Session, TimeoutConfig, WaitFormat};
use tokio::net::TcpListener;

async fn dead_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

fn plain() -> Session {
    Session::builder()
        .retry(RetryPolicy::none())
        .build()
        .unwrap()
}

#[tokio::test]
async fn transient_returns_a_long_server_wait_instead_of_sleeping() {
    let server = TestServer::http(queue([
        TestResponse::new(429).close().header("retry-after", "3600"),
        TestResponse::new(200).close(),
    ]))
    .await
    .unwrap();
    let session = Session::builder()
        .retry(RetryPolicy::transient())
        .timeout(TimeoutConfig::new().total(None))
        .build()
        .unwrap();
    let resp = tokio::time::timeout(Duration::from_secs(5), session.get(server.url("/")).send())
        .await
        .expect("slept for the server wait")
        .unwrap();
    assert_eq!(resp.status().as_u16(), 429);
    assert_eq!(server.requests().await.len(), 1);
}

#[test]
fn backoff_saturates_instead_of_panicking() {
    let policy = RetryPolicy::transient()
        .initial_backoff(Duration::from_secs(1))
        .max_backoff(Duration::MAX)
        .backoff_factor(2.0)
        .jitter(false);
    assert_eq!(policy.backoff(2000), Duration::MAX);
}

#[tokio::test]
async fn a_configured_wait_header_wins_over_retry_after() {
    let server = TestServer::http(queue([
        TestResponse::new(429)
            .close()
            .header("retry-after", "100")
            .header("x-wait", "0"),
        TestResponse::new(200).close(),
    ]))
    .await
    .unwrap();
    let session = Session::builder()
        .retry(
            RetryPolicy::transient()
                .max_retry_after(Duration::from_secs(5))
                .wait_header("x-wait", WaitFormat::Seconds),
        )
        .build()
        .unwrap();
    let resp = session.get(server.url("/")).await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(server.requests().await.len(), 2);
}

#[tokio::test]
async fn a_declined_retry_counts_as_exhausted() {
    let server = TestServer::http(queue([TestResponse::new(503)
        .close()
        .header("retry-after", "100")]))
    .await
    .unwrap();
    let err = Session::builder()
        .retry(RetryPolicy::transient().max_retry_after(Duration::from_secs(1)))
        .build()
        .unwrap()
        .get(server.url("/"))
        .error_for_status()
        .send()
        .await
        .unwrap_err();
    assert_eq!(err.status().map(|s| s.as_u16()), Some(503));
    assert!(err.retries_exhausted());
}

#[tokio::test]
async fn an_unsent_streaming_body_is_retried() {
    let proxy = TestServer::http(queue([TestResponse::new(200).close()]))
        .await
        .unwrap();
    let session = Session::builder()
        .proxy(format!("http://127.0.0.1:{}", dead_port().await))
        .retry(
            RetryPolicy::transient()
                .initial_backoff(Duration::from_millis(1))
                .jitter(false)
                .retry_unsent(true)
                .rotate_proxies([format!("http://{}", proxy.addr())]),
        )
        .build()
        .unwrap();
    let chunks = futures_util::stream::iter([
        Ok::<_, std::io::Error>(Bytes::from_static(b"part-one,")),
        Ok(Bytes::from_static(b"part-two")),
    ]);
    let resp = session
        .post("http://origin.test/upload")
        .body(Body::stream(chunks, None))
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let sent = proxy.next_request().await.unwrap();
    assert_eq!(sent.target, "http://origin.test/upload");
    assert_eq!(sent.body, b"part-one,part-two");
}

#[tokio::test]
async fn download_keeps_the_same_error_body_as_send() {
    let body = vec![b'e'; 10 * 1024];
    let server = TestServer::http(queue([TestResponse::new(503).close().body(body.clone())]))
        .await
        .unwrap();
    let dir = std::env::temp_dir().join(format!("leyline-errbody-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let err = plain()
        .get(server.url("/file"))
        .download(dir.join("file.bin"), None)
        .await
        .unwrap_err();
    assert_eq!(err.body(), Some(body.as_slice()));
    drop(std::fs::remove_dir_all(&dir));
}

#[tokio::test]
async fn an_error_body_bomb_stops_at_the_error_body_cap() {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(&vec![0u8; 4 << 20]).unwrap();
    let bomb = encoder.finish().unwrap();
    let server = TestServer::http(queue([TestResponse::new(503)
        .close()
        .header("content-encoding", "gzip")
        .body(bomb)]))
    .await
    .unwrap();
    let dir = std::env::temp_dir().join(format!("leyline-bomb-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let err = Session::builder()
        .retry(RetryPolicy::none())
        .compression(CompressionConfig::new().max_body_size(1 << 20))
        .build()
        .unwrap()
        .get(server.url("/"))
        .download(dir.join("bomb.bin"), None)
        .await
        .unwrap_err();
    drop(std::fs::remove_dir_all(&dir));
    assert_eq!(err.status().map(|s| s.as_u16()), Some(503));
    assert_eq!(err.body().map(<[u8]>::len), Some(64 * 1024));
}

#[tokio::test]
async fn the_body_timeout_starts_at_the_first_read() {
    let server = TestServer::http(queue([TestResponse::new(200).close().body("fast")]))
        .await
        .unwrap();
    let resp = plain()
        .get(server.url("/"))
        .timeout(TimeoutConfig::new().body(Duration::from_millis(300)))
        .stream()
        .send()
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(resp.bytes().await.unwrap(), "fast");
}
