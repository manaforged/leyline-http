#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use crate::h2_support as support;
use crate::tls_support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

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

async fn refuse_first_request(mut stream: SslStream<TcpStream>, seen: Arc<AtomicUsize>) {
    read_preface(&mut stream).await;
    write_server_settings(&mut stream).await;
    loop {
        let (frame, _) = read_frame(&mut stream).await;
        if frame.frame_type == FrameType::Settings as u8 && frame.flags & SETTINGS_ACK == 0 {
            write_settings_ack(&mut stream).await;
        }
        if frame.frame_type != FrameType::Data as u8 && frame.frame_type != FrameType::Headers as u8
        {
            continue;
        }
        if frame.flags & END_STREAM == 0 {
            continue;
        }
        if seen.fetch_add(1, Ordering::SeqCst) == 0 {
            let mut rst = vec![0, 0, 4, FrameType::RstStream as u8, 0];
            rst.extend_from_slice(&frame.stream_id.to_be_bytes());
            rst.extend_from_slice(&REFUSED_STREAM.to_be_bytes());
            stream.write_all(&rst).await.unwrap();
            continue;
        }
        write_response_headers(&mut stream, frame.stream_id).await;
        write_data(&mut stream, frame.stream_id, b"ok", true).await;
    }
}

#[tokio::test]
async fn retry_unsent_resends_a_refused_post() {
    let (cert, key) = tls_support::self_signed();
    let der = cert.to_der().unwrap();
    let seen = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&seen);
    let port = tls_support::tls_server(cert, key, Arc::new(AtomicUsize::new(0)), move |s| {
        refuse_first_request(s, Arc::clone(&counter))
    })
    .await;
    let session = Session::builder()
        .tls_trust(
            TlsTrustConfig::new()
                .env_roots(false)
                .system_roots(false)
                .add_ca_der(der),
        )
        .retry(
            RetryPolicy::transient()
                .initial_backoff(Duration::from_millis(1))
                .jitter(false)
                .retry_unsent(true),
        )
        .build()
        .unwrap();
    let resp = session
        .post(format!("https://127.0.0.1:{port}/order"))
        .body("item=1")
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(seen.load(Ordering::SeqCst), 2);
}
