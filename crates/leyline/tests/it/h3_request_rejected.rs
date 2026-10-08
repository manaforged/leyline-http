#![cfg(feature = "http3")]
use crate::h3_support;

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

#[tokio::test]
async fn retry_unsent_resends_a_post_rejected_twice() {
    let rejected = || Reply::Reset(WireErrorCode::RequestRejected as u64);
    let server = h3_server(
        vec![rejected(), rejected(), Reply::Body(LENGTH)],
        Limits::default(),
    )
    .await;
    let session = server
        .session()
        .retry(
            leyline::RetryPolicy::transient()
                .initial_backoff(std::time::Duration::from_millis(1))
                .jitter(false)
                .retry_unsent(true),
        )
        .build()
        .unwrap();
    let body = session
        .post(server.url())
        .body("order=1")
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(body.len(), LENGTH);
}

#[tokio::test]
async fn a_twice_rejected_request_keeps_the_pooled_connection() {
    let rejected = || Reply::Reset(WireErrorCode::RequestRejected as u64);
    let server = h3_server(
        vec![
            Reply::Body(LENGTH),
            rejected(),
            rejected(),
            Reply::Body(LENGTH),
        ],
        Limits::default(),
    )
    .await;
    let session = server
        .session()
        .retry(
            leyline::RetryPolicy::transient()
                .initial_backoff(std::time::Duration::from_millis(1))
                .jitter(false)
                .retry_unsent(true),
        )
        .build()
        .unwrap();
    session
        .get(server.url())
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let body = session
        .post(server.url())
        .body("order=1")
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(body.len(), LENGTH);
    assert_eq!(server.connections(), 1);
}
