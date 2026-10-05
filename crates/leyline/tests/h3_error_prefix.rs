#![cfg(feature = "http3")]
#[path = "h3_support/mod.rs"]
mod h3_support;
#[path = "tls_support/mod.rs"]
mod tls_support;

use std::time::{Duration, Instant};

use h3_support::{Limits, Reply, h3_server};
use leyline::TimeoutConfig;

#[tokio::test]
async fn an_h3_error_with_a_stalled_body_returns_at_the_error_body_timeout() {
    let server = h3_server(vec![Reply::Status(b"500")], Limits::default()).await;
    let session = server
        .session()
        .timeout(
            TimeoutConfig::new()
                .total(Duration::from_secs(20))
                .error_body(Duration::from_millis(300)),
        )
        .build()
        .unwrap();

    let started = Instant::now();
    let err = session
        .get(server.url())
        .error_for_status()
        .await
        .unwrap_err();

    assert_eq!(err.status().map(|s| s.as_u16()), Some(500), "{err}");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn an_h3_success_under_error_for_status_is_buffered() {
    let server = h3_server(vec![Reply::Body(16)], Limits::default()).await;
    let session = server.session().build().unwrap();

    let response = session.get(server.url()).error_for_status().await.unwrap();

    assert_eq!(response.bytes().await.unwrap().len(), 16);
}
