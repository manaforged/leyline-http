#![cfg(feature = "http3")]
#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use crate::h3_support;

use futures_util::StreamExt;
use h3_support::{H3Server, Limits, Reply, h3_server};
use leyline::{CompressionConfig, Kind, Session};

const CAP: usize = 1024;
const SMALL: usize = 16;
const LARGE: usize = 4096;

fn capped(server: &H3Server) -> Session {
    server
        .session()
        .compression(CompressionConfig::new().max_body_size(CAP))
        .build()
        .unwrap()
}

#[tokio::test]
async fn an_h3_body_over_the_cap_is_a_body_error() {
    let server = h3_server(
        vec![Reply::Body(SMALL), Reply::Body(LARGE)],
        Limits::default(),
    )
    .await;
    let session = capped(&server);
    let url = server.url();
    let first = session.get(&url).await.unwrap().bytes().await.unwrap();
    assert_eq!(first.len(), SMALL);
    let second = async { session.get(&url).await?.bytes().await }.await;
    let err = second.unwrap_err();
    assert_eq!(err.kind(), Kind::Body, "{err:?}");
}

#[tokio::test]
async fn a_streamed_h3_body_is_not_capped() {
    let server = h3_server(vec![Reply::Body(LARGE)], Limits::default()).await;
    let session = capped(&server);
    let url = server.url();
    let mut body = session
        .get(&url)
        .stream()
        .await
        .unwrap()
        .into_stream()
        .unwrap();
    let mut read = 0;
    while let Some(chunk) = body.next().await {
        read += chunk.unwrap().len();
    }
    assert_eq!(read, LARGE);
}
