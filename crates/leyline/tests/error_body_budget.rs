use std::time::{Duration, Instant};

use leyline::testing::{TestResponse, TestServer, queue};
use leyline::{CompressionConfig, Session, TimeoutConfig};

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
}
