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

use futures_util::StreamExt;
use h3_support::{Reply, h3_server};
use leyline_quiche::h3::WireErrorCode;

const CHUNK: usize = 1024;

#[tokio::test]
async fn a_dropped_response_body_stops_the_stream_as_request_cancelled() {
    let server = h3_server(vec![Reply::Hold(CHUNK)]).await;
    let session = server.session().build().unwrap();
    let mut body = session
        .get(server.url())
        .stream()
        .await
        .unwrap()
        .into_stream()
        .unwrap();
    body.next().await.unwrap().unwrap();
    drop(body);
    assert_eq!(
        server.stop_code().await,
        Some(WireErrorCode::RequestCancelled as u64)
    );
}

#[tokio::test]
async fn a_malformed_response_head_stops_the_stream_as_a_protocol_error() {
    let server = h3_server(vec![Reply::Status(b"abc")]).await;
    let session = server.session().build().unwrap();
    session.get(server.url()).await.unwrap_err();
    assert_eq!(
        server.stop_code().await,
        Some(WireErrorCode::GeneralProtocolError as u64)
    );
}

#[tokio::test]
async fn dropping_an_idle_session_sends_no_connection_close() {
    let server = h3_server(vec![Reply::Body(CHUNK)]).await;
    let session = server.session().build().unwrap();
    session
        .get(server.url())
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    drop(session);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(server.peer_close(), None);
}
