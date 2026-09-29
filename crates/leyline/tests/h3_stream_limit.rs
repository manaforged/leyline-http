#![cfg(feature = "http3")]
#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
#[path = "h3_support/mod.rs"]
mod h3_support;
#[path = "tls_support/mod.rs"]
mod tls_support;

use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use h3_support::{Limits, Reply, h3_server};
use leyline::{Body, Session};
use leyline_quiche::h3::WireErrorCode;

const LENGTH: usize = 4096;
const ONE_STREAM: Limits = Limits { streams: 1 };
const CREDIT: Duration = Duration::from_secs(5);

async fn fetch(session: &Session, url: &str) -> usize {
    session.get(url).await.unwrap().bytes().await.unwrap().len()
}

async fn assert_credit_returned(session: &Session, url: &str) {
    let read = tokio::time::timeout(CREDIT, fetch(session, url)).await;
    assert_eq!(
        read.ok(),
        Some(LENGTH),
        "the next request never got a stream"
    );
}

fn open_upload() -> Body {
    let first = futures_util::stream::iter([Ok::<_, std::io::Error>(Bytes::from_static(b"part"))]);
    Body::stream(first.chain(futures_util::stream::pending()), None)
}

#[tokio::test]
async fn requests_past_the_stream_limit_wait_for_credit() {
    let server = h3_server(vec![Reply::Body(LENGTH)], ONE_STREAM).await;
    let session = server.session().build().unwrap();
    let url = server.url();
    assert_eq!(fetch(&session, &url).await, LENGTH);
    let (first, second) = tokio::join!(fetch(&session, &url), fetch(&session, &url));
    assert_eq!((first, second), (LENGTH, LENGTH));
}

#[tokio::test]
async fn a_reset_during_the_upload_returns_the_stream_credit() {
    let reset = Reply::ResetResponse(WireErrorCode::InternalError as u64);
    let server = h3_server(vec![reset, Reply::Body(LENGTH)], ONE_STREAM).await;
    let session = server.session().build().unwrap();
    let url = server.url();
    session.post(&url).body(open_upload()).await.unwrap_err();
    assert_credit_returned(&session, &url).await;
}

#[tokio::test]
async fn a_response_that_ends_before_the_upload_returns_the_stream_credit() {
    let server = h3_server(vec![Reply::Body(LENGTH)], ONE_STREAM).await;
    let session = server.session().build().unwrap();
    let url = server.url();
    let mut body = session
        .post(&url)
        .body(open_upload())
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
    assert_credit_returned(&session, &url).await;
}
