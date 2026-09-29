#![cfg(feature = "http3")]
#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
#[path = "h3_support/mod.rs"]
mod h3_support;
#[path = "tls_support/mod.rs"]
mod tls_support;

use futures_util::StreamExt;
use h3_support::{Limits, Reply, h3_server};
use leyline_quiche::h3::WireErrorCode;

const LENGTH: usize = 4096;

#[tokio::test]
async fn a_rejected_request_streams_the_response_to_its_retry() {
    let rejected = Reply::Reset(WireErrorCode::RequestRejected as u64);
    let server = h3_server(vec![rejected, Reply::Body(LENGTH)], Limits::default()).await;
    let mut body = server
        .session()
        .build()
        .unwrap()
        .get(server.url())
        .stream()
        .await
        .unwrap()
        .into_stream()
        .unwrap();
    let mut read = 0;
    while let Some(chunk) = body.next().await {
        read += chunk.unwrap().len();
    }
    assert_eq!(read, LENGTH);
}

#[tokio::test]
async fn a_rejected_request_after_goaway_is_resent_on_a_new_connection() {
    let rejected = Reply::GoawayThenReset(WireErrorCode::RequestRejected as u64);
    let replies = vec![Reply::Body(LENGTH), rejected, Reply::Body(LENGTH)];
    let server = h3_server(replies, Limits::default()).await;
    let session = server.session().build().unwrap();
    for _ in 0..2 {
        let body = session
            .get(server.url())
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(body.len(), LENGTH);
    }
}

#[tokio::test]
async fn a_rejection_after_the_response_head_is_not_resent() {
    let rejected = Reply::Truncate(0, WireErrorCode::RequestRejected as u64);
    let server = h3_server(vec![rejected, Reply::Body(LENGTH)], Limits::default()).await;
    let session = server.session().build().unwrap();
    session.get(server.url()).await.unwrap_err();
}
