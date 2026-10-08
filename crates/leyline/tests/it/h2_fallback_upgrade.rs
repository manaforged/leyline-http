#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use crate::h2_support as support;
use crate::tls_support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use leyline::h2::frame::FrameType;
use leyline::{RetryPolicy, Session, TlsTrustConfig};
use leyline_bssl_tokio::SslStream;
use support::{
    read_frame, read_preface, write_data, write_response_headers, write_server_settings,
    write_settings_ack,
};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

const SETTINGS_ACK: u8 = 0x1;
const END_STREAM: u8 = 0x1;
const REFUSED_STREAM: u32 = 0x7;

async fn serve_h2(mut stream: SslStream<TcpStream>, refuse_after: Option<usize>) {
    read_preface(&mut stream).await;
    write_server_settings(&mut stream).await;
    let mut answered = 0;
    loop {
        let (frame, _) = read_frame(&mut stream).await;
        if frame.frame_type == FrameType::Settings as u8 && frame.flags & SETTINGS_ACK == 0 {
            write_settings_ack(&mut stream).await;
        }
        if frame.frame_type != FrameType::Headers as u8 || frame.flags & END_STREAM == 0 {
            continue;
        }
        if refuse_after.is_some_and(|limit| answered >= limit) {
            let mut rst = vec![0, 0, 4, FrameType::RstStream as u8, 0];
            rst.extend_from_slice(&frame.stream_id.to_be_bytes());
            rst.extend_from_slice(&REFUSED_STREAM.to_be_bytes());
            stream.write_all(&rst).await.unwrap();
            continue;
        }
        write_response_headers(&mut stream, frame.stream_id).await;
        write_data(&mut stream, frame.stream_id, b"ok", true).await;
        answered += 1;
    }
}

#[tokio::test]
async fn an_http1_fallback_that_reaches_h2_sends_on_h2() {
    let (cert, key) = tls_support::self_signed();
    let der = cert.to_der().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let alpn = |index: usize| {
        if index == 1 {
            tls_support::HTTP11
        } else {
            tls_support::H2
        }
    };
    let port = tls_support::tls_server_per_connection(
        cert,
        key,
        Arc::clone(&connections),
        alpn,
        |index, stream| async move {
            match index {
                0 => serve_h2(stream, Some(1)).await,
                1 => drop(stream),
                _ => serve_h2(stream, None).await,
            }
        },
    )
    .await;
    let session = Session::builder()
        .tls_trust(
            TlsTrustConfig::new()
                .env_roots(false)
                .system_roots(false)
                .add_ca_der(der),
        )
        .retry(RetryPolicy::none())
        .build()
        .unwrap();
    let url = format!("https://127.0.0.1:{port}/");
    session
        .get(format!("{url}a"))
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let resp = session.get(format!("{url}b")).await.unwrap();
    assert_eq!(resp.version(), leyline::HttpVersion::Http2);
    assert_eq!(resp.bytes().await.unwrap(), "ok");
    assert_eq!(connections.load(Ordering::SeqCst), 3);
}
