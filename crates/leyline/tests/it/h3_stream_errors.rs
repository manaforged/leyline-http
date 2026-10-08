#![cfg(feature = "http3")]
use crate::h3_support;

use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use h3_support::{Limits, Reply, h3_server};
use leyline::{Body, Kind};
use leyline_quiche::h3::WireErrorCode;

const CHUNK: usize = 1024;
const TRUNCATED: usize = 256 * 1024;

#[tokio::test]
async fn a_dropped_response_body_stops_the_stream_as_request_cancelled() {
    let server = h3_server(vec![Reply::Hold(CHUNK)], Limits::default()).await;
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
    let server = h3_server(vec![Reply::Status(b"abc")], Limits::default()).await;
    let session = server.session().build().unwrap();
    session.get(server.url()).await.unwrap_err();
    assert_eq!(
        server.stop_code().await,
        Some(WireErrorCode::GeneralProtocolError as u64)
    );
}

#[tokio::test]
async fn dropping_an_idle_session_sends_no_connection_close() {
    let server = h3_server(vec![Reply::Body(CHUNK)], Limits::default()).await;
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

#[tokio::test]
async fn a_reset_in_the_middle_of_a_streamed_body_is_an_error() {
    let truncated = Reply::Truncate(TRUNCATED, WireErrorCode::InternalError as u64);
    let server = h3_server(vec![truncated], Limits::default()).await;
    let session = server.session().build().unwrap();
    let mut body = session
        .get(server.url())
        .stream()
        .await
        .unwrap()
        .into_stream()
        .unwrap();
    let mut failed = false;
    while let Some(chunk) = body.next().await {
        if chunk.is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed, "a reset body must not end as if complete");
}

#[tokio::test]
async fn a_failing_request_body_stream_is_a_body_error() {
    let server = h3_server(vec![Reply::Hold(CHUNK)], Limits::default()).await;
    let session = server.session().build().unwrap();
    let chunks =
        futures_util::stream::iter([Err::<Bytes, _>(std::io::Error::other("caller body failed"))]);
    let err = session
        .post(server.url())
        .body(Body::stream(chunks, None))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Body, "{err:?}");
    assert_eq!(
        err.io().map(ToString::to_string).as_deref(),
        Some("caller body failed"),
        "{err:?}"
    );
}

#[tokio::test]
async fn an_h3_upload_must_match_its_declared_length() {
    const DECLARED: usize = 10;
    for produced in [DECLARED - 1, DECLARED + 1] {
        let server = h3_server(vec![Reply::Body(CHUNK)], Limits::default()).await;
        let session = server.session().build().unwrap();
        let chunks = [Ok::<_, std::io::Error>(Bytes::from(vec![b'u'; produced]))];
        let body = Body::stream(futures_util::stream::iter(chunks), Some(DECLARED as u64));
        let sent = session.post(server.url()).body(body).await;
        assert!(
            sent.is_err(),
            "{produced} bytes under a declared {DECLARED}: {sent:?}"
        );
    }
}

#[tokio::test]
async fn a_failed_decoded_h3_stream_that_is_kept_releases_its_host_slot() {
    let server = h3_server(vec![Reply::Encoded(CHUNK)], Limits::default()).await;
    let session = server
        .session()
        .host_limits(leyline::HostLimits::new().max_in_flight(1))
        .build()
        .unwrap();
    let mut body = session
        .get(server.url())
        .stream()
        .await
        .unwrap()
        .into_decoded_stream(None)
        .unwrap();
    let mut failed = false;
    while let Some(chunk) = body.next().await {
        if chunk.is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed, "a body that is not gzip must fail to decode");
    assert!(session.host_stats().iter().all(|s| s.in_flight() == 0));
    assert_eq!(
        server.stop_code().await,
        Some(WireErrorCode::RequestCancelled as u64)
    );
    drop(body);
}

#[tokio::test]
async fn an_upload_that_fails_behind_a_full_unread_body_leaves_no_task_waiting_on_it() {
    let server = h3_server(vec![Reply::Flood(8 * 1024 * 1024)], Limits::default()).await;
    let session = server.session().build().unwrap();
    let (fail_tx, fail_rx) = tokio::sync::oneshot::channel::<()>();
    let first = futures_util::stream::iter([Ok::<_, std::io::Error>(Bytes::from_static(b"part"))]);
    let failure = futures_util::stream::once(async move {
        drop(fail_rx.await);
        Err::<Bytes, _>(std::io::Error::other(
            "upload failed after the response head",
        ))
    });
    let body = session
        .post(server.url())
        .body(Body::stream(first.chain(failure), None))
        .stream()
        .await
        .unwrap()
        .into_stream()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    fail_tx.send(()).unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let metrics = tokio::runtime::Handle::current().metrics();
    let retained = metrics.num_alive_tasks();
    drop(body);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        metrics.num_alive_tasks(),
        retained,
        "a task was waiting to deliver the upload error into the unread body"
    );
}
