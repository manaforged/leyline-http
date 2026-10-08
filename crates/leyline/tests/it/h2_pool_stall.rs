use crate::h2_support as support;
use crate::tls_support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use leyline::h2::frame::{FRAME_HEADER_LEN, FrameHeader, FrameType};
use leyline::{Session, TimeoutConfig, TlsTrustConfig};
use leyline_bssl_tokio::SslStream;
use support::{
    read_preface, write_data, write_raw_headers, write_server_settings, write_server_settings_with,
    write_settings_ack, write_window_update,
};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::TcpStream;

const SETTINGS_ACK: u8 = 0x1;
const INITIAL_WINDOW_SIZE: u16 = 0x4;
const WIDE: u32 = 1 << 30;

async fn next_frame<S: AsyncRead + Unpin>(stream: &mut S) -> Option<FrameHeader> {
    let mut head = [0u8; FRAME_HEADER_LEN];
    stream.read_exact(&mut head).await.ok()?;
    let header = FrameHeader::parse(&head);
    let mut payload = vec![0u8; header.length as usize];
    stream.read_exact(&mut payload).await.ok()?;
    Some(header)
}

async fn stops_reading(mut stream: SslStream<TcpStream>) {
    read_preface(&mut stream).await;
    write_server_settings_with(&mut stream, vec![(INITIAL_WINDOW_SIZE, WIDE)]).await;
    write_window_update(&mut stream, 0, WIDE).await;
    let mut answered = 0;
    while let Some(frame) = next_frame(&mut stream).await {
        if frame.frame_type == FrameType::Settings as u8 && frame.flags & SETTINGS_ACK == 0 {
            write_settings_ack(&mut stream).await;
        }
        if frame.frame_type != FrameType::Headers as u8 {
            continue;
        }
        answered += 1;
        if answered == 1 {
            write_raw_headers(&mut stream, frame.stream_id, &[(":status", "200")], false).await;
            write_data(&mut stream, frame.stream_id, b"x", false).await;
        } else {
            tokio::time::sleep(Duration::from_secs(30)).await;
            return;
        }
    }
}

async fn answers(mut stream: SslStream<TcpStream>) {
    read_preface(&mut stream).await;
    write_server_settings(&mut stream).await;
    while let Some(frame) = next_frame(&mut stream).await {
        if frame.frame_type == FrameType::Settings as u8 && frame.flags & SETTINGS_ACK == 0 {
            write_settings_ack(&mut stream).await;
        }
        if frame.frame_type == FrameType::Headers as u8 {
            write_raw_headers(&mut stream, frame.stream_id, &[(":status", "200")], false).await;
            write_data(&mut stream, frame.stream_id, b"fresh", true).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn new_requests_leave_a_connection_whose_peer_stopped_reading() {
    let (cert, key) = tls_support::self_signed();
    let der = cert.to_der().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let port = tls_support::tls_server_per_connection(
        cert,
        key,
        Arc::clone(&connections),
        |_| tls_support::H2,
        |index, stream| async move {
            if index == 0 {
                stops_reading(stream).await;
            } else {
                answers(stream).await;
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
        .build()
        .unwrap();
    let url = format!("https://127.0.0.1:{port}/");
    let retained = session.get(&url).stream().await.unwrap();

    let blocked = session
        .post(&url)
        .body(vec![0u8; 16 * 1024 * 1024])
        .timeout(TimeoutConfig::new().total(Duration::from_millis(500)))
        .send()
        .await;
    assert!(blocked.is_err(), "{blocked:?}");
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let next = session
        .get(&url)
        .timeout(TimeoutConfig::new().total(Duration::from_secs(3)))
        .send()
        .await
        .unwrap();
    assert_eq!(next.text().await.unwrap(), "fresh");
    assert_eq!(connections.load(Ordering::SeqCst), 2);
    drop(retained);
}
