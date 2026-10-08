#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use crate::h2_support as support;
use crate::tls_support;

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bytes::Bytes;
use leyline::h2::frame::FrameType;
use leyline::{Body, Kind, Session, TlsTrustConfig};
use leyline_bssl_tokio::SslStream;
use support::{
    read_frame, read_preface, write_data, write_response_headers, write_server_settings,
    write_settings_ack,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

const SETTINGS_ACK: u8 = 0x1;
const END_STREAM: u8 = 0x1;

async fn answer_once_then_drop<S: AsyncRead + AsyncWrite + Unpin>(mut stream: S) {
    read_preface(&mut stream).await;
    write_server_settings(&mut stream).await;
    let mut answered = false;
    loop {
        let (frame, _) = read_frame(&mut stream).await;
        if frame.frame_type == FrameType::Settings as u8 && frame.flags & SETTINGS_ACK == 0 {
            write_settings_ack(&mut stream).await;
        }
        if frame.frame_type != FrameType::Headers as u8 {
            continue;
        }
        if answered {
            return;
        }
        write_response_headers(&mut stream, frame.stream_id).await;
        write_data(&mut stream, frame.stream_id, b"ok", true).await;
        answered = true;
    }
}

async fn answer_finished_requests<S: AsyncRead + AsyncWrite + Unpin>(mut stream: S) {
    read_preface(&mut stream).await;
    write_server_settings(&mut stream).await;
    loop {
        let (frame, _) = read_frame(&mut stream).await;
        if frame.frame_type == FrameType::Settings as u8 && frame.flags & SETTINGS_ACK == 0 {
            write_settings_ack(&mut stream).await;
        }
        if frame.frame_type == FrameType::Headers as u8 && frame.flags & END_STREAM != 0 {
            write_response_headers(&mut stream, frame.stream_id).await;
            write_data(&mut stream, frame.stream_id, b"ok", true).await;
        }
    }
}

async fn start<F, Fut>(serve: F) -> (Session, String, Arc<AtomicUsize>)
where
    F: Fn(SslStream<TcpStream>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (cert, key) = tls_support::self_signed();
    let der = cert.to_der().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let port = tls_support::tls_server(cert, key, Arc::clone(&connections), serve).await;
    let session = Session::builder()
        .tls_trust(
            TlsTrustConfig::new()
                .env_roots(false)
                .system_roots(false)
                .add_ca_der(der),
        )
        .build()
        .unwrap();
    (session, format!("https://127.0.0.1:{port}/"), connections)
}

#[tokio::test]
async fn a_streaming_body_on_a_dead_pooled_h2_connection_is_a_body_error() {
    let (session, url, connections) = start(answer_once_then_drop).await;
    session.get(&url).await.unwrap().bytes().await.unwrap();
    let chunks =
        futures_util::stream::iter([Ok::<_, std::io::Error>(Bytes::from_static(b"payload"))]);
    let err = session
        .put(&url)
        .body(Body::stream(chunks, None))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Body, "{err:?}");
    assert_eq!(connections.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_failing_request_body_stream_keeps_the_pooled_h2_connection() {
    let (session, url, connections) = start(answer_finished_requests).await;
    session.get(&url).await.unwrap().bytes().await.unwrap();
    let chunks =
        futures_util::stream::iter([Err::<Bytes, _>(std::io::Error::other("caller body failed"))]);
    let err = session
        .put(&url)
        .body(Body::stream(chunks, None))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Body, "{err:?}");
    session.get(&url).await.unwrap().bytes().await.unwrap();
    assert_eq!(connections.load(Ordering::SeqCst), 1);
}
