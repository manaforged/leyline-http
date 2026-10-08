use crate::core_support::forward;

use std::io::Write;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use forward::counting_forwarder;
use leyline::testing::{TestResponse, TestServer, queue};
use leyline::{CompressionConfig, Session, TimeoutConfig};

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

#[tokio::test]
async fn an_error_body_that_inflates_past_the_body_limit_is_a_status_error() {
    let server = TestServer::http(queue([TestResponse::new(500)
        .header("content-encoding", "gzip")
        .body(gzip(&vec![b'x'; 4 << 20]))]))
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
    assert_eq!(err.body(), Some(&vec![b'x'; 64 * 1024][..]));
    assert_eq!(err.header("content-encoding"), None);
    assert_eq!(err.header("content-length"), None);
}

#[tokio::test]
async fn error_for_status_reuses_the_connection_across_a_redirect() {
    let server = TestServer::http(queue([
        TestResponse::new(302)
            .header("location", "/final")
            .body("moved"),
        TestResponse::new(200).body("ok"),
    ]))
    .await
    .unwrap();
    let (port, accepted) = counting_forwarder(server.addr()).await;
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
    assert_eq!(err.header("content-length"), None);
}

#[tokio::test]
async fn error_for_status_reuses_the_connection_across_a_status_retry() {
    let server = TestServer::http(queue([
        TestResponse::new(503)
            .header("retry-after", "0")
            .body("busy"),
        TestResponse::new(200).body("ok"),
    ]))
    .await
    .unwrap();
    let (port, accepted) = counting_forwarder(server.addr()).await;
    let session = Session::builder()
        .retry(
            leyline::RetryPolicy::transient()
                .initial_backoff(Duration::from_millis(1))
                .jitter(false),
        )
        .build()
        .unwrap();

    let response = session
        .get(format!("http://127.0.0.1:{port}/"))
        .error_for_status()
        .await
        .unwrap();

    assert_eq!(response.text().await.unwrap(), "ok");
    assert_eq!(accepted.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn error_for_status_reuses_the_connection_across_a_digest_challenge() {
    let server = TestServer::http(queue([
        TestResponse::new(401)
            .header(
                "www-authenticate",
                "Digest realm=\"r\", nonce=\"n1\", qop=\"auth\", algorithm=MD5",
            )
            .body("denied"),
        TestResponse::new(200).body("ok"),
    ]))
    .await
    .unwrap();
    let (port, accepted) = counting_forwarder(server.addr()).await;
    let session = Session::builder().build().unwrap();

    let response = session
        .get(format!("http://127.0.0.1:{port}/"))
        .digest_auth(leyline::DigestAuth::new("user", "pass"))
        .error_for_status()
        .await
        .unwrap();

    assert_eq!(response.text().await.unwrap(), "ok");
    assert_eq!(accepted.load(Ordering::SeqCst), 1);
}
