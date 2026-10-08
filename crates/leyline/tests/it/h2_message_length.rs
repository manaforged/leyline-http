#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use crate::h2_support as support;
use crate::tls_support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bytes::Bytes;
use leyline::h2::frame::{FRAME_HEADER_LEN, FrameHeader, FrameType};
use leyline::{Body, Session, TlsTrustConfig};
use support::{
    read_preface, write_data, write_raw_headers, write_server_settings, write_settings_ack,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};

const SETTINGS_ACK: u8 = 0x1;
const END_STREAM: u8 = 0x1;
const DECLARED: usize = 10;

async fn next_frame<S: AsyncRead + Unpin>(stream: &mut S) -> Option<(FrameHeader, Vec<u8>)> {
    let mut head = [0u8; FRAME_HEADER_LEN];
    stream.read_exact(&mut head).await.ok()?;
    let header = FrameHeader::parse(&head);
    let mut payload = vec![0u8; header.length as usize];
    stream.read_exact(&mut payload).await.ok()?;
    Some((header, payload))
}

async fn trailers_after<S: AsyncRead + AsyncWrite + Unpin>(mut stream: S, sent: usize) {
    read_preface(&mut stream).await;
    write_server_settings(&mut stream).await;
    while let Some((frame, _)) = next_frame(&mut stream).await {
        if frame.frame_type == FrameType::Settings as u8 && frame.flags & SETTINGS_ACK == 0 {
            write_settings_ack(&mut stream).await;
        }
        if frame.frame_type != FrameType::Headers as u8 {
            continue;
        }
        let declared = DECLARED.to_string();
        write_raw_headers(
            &mut stream,
            frame.stream_id,
            &[(":status", "200"), ("content-length", &declared)],
            false,
        )
        .await;
        write_data(&mut stream, frame.stream_id, &vec![b'x'; sent], false).await;
        write_raw_headers(&mut stream, frame.stream_id, &[("x-check", "1")], true).await;
    }
}

async fn counting_upload<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    received: Arc<AtomicUsize>,
) {
    read_preface(&mut stream).await;
    write_server_settings(&mut stream).await;
    while let Some((frame, payload)) = next_frame(&mut stream).await {
        if frame.frame_type == FrameType::Settings as u8 && frame.flags & SETTINGS_ACK == 0 {
            write_settings_ack(&mut stream).await;
        }
        if frame.frame_type == FrameType::Data as u8 {
            received.fetch_add(payload.len(), Ordering::SeqCst);
            if frame.flags & END_STREAM != 0 {
                write_raw_headers(&mut stream, frame.stream_id, &[(":status", "200")], true).await;
            }
        }
    }
}

fn session_for(der: Vec<u8>) -> Session {
    Session::builder()
        .tls_trust(
            TlsTrustConfig::new()
                .env_roots(false)
                .system_roots(false)
                .add_ca_der(der),
        )
        .build()
        .unwrap()
}

async fn h2_server<F, Fut>(serve: F) -> (String, Session)
where
    F: Fn(leyline_bssl_tokio::SslStream<tokio::net::TcpStream>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let (cert, key) = tls_support::self_signed();
    let der = cert.to_der().unwrap();
    let port = tls_support::tls_server(cert, key, Arc::new(AtomicUsize::new(0)), serve).await;
    (format!("https://127.0.0.1:{port}/"), session_for(der))
}

#[tokio::test]
async fn trailers_end_a_response_only_when_its_length_matches() {
    let (short_url, session) = h2_server(|stream| trailers_after(stream, DECLARED / 2)).await;
    let short = async { session.get(&short_url).await?.bytes().await }.await;
    assert!(short.is_err(), "{short:?}");

    let (full_url, session) = h2_server(|stream| trailers_after(stream, DECLARED)).await;
    let full = session.get(&full_url).await.unwrap();
    assert_eq!(full.bytes().await.unwrap().len(), DECLARED);
}

#[tokio::test]
async fn an_h2_upload_must_match_its_declared_length() {
    for produced in [DECLARED - 1, DECLARED + 1] {
        let received = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&received);
        let (url, session) =
            h2_server(move |stream| counting_upload(stream, Arc::clone(&counter))).await;
        let chunks = vec![Ok::<_, std::io::Error>(Bytes::from(vec![b'u'; produced]))];
        let body = Body::stream(futures_util::stream::iter(chunks), Some(DECLARED as u64));
        let sent = session.post(&url).body(body).send().await;
        assert!(
            sent.is_err(),
            "{produced} bytes under a declared {DECLARED}: {sent:?}"
        );
        assert!(received.load(Ordering::SeqCst) <= DECLARED);
    }
}

async fn not_gzip<S: AsyncRead + AsyncWrite + Unpin>(mut stream: S, resets: Arc<AtomicUsize>) {
    read_preface(&mut stream).await;
    write_server_settings(&mut stream).await;
    while let Some((frame, _)) = next_frame(&mut stream).await {
        if frame.frame_type == FrameType::Settings as u8 && frame.flags & SETTINGS_ACK == 0 {
            write_settings_ack(&mut stream).await;
        }
        if frame.frame_type == FrameType::RstStream as u8 {
            resets.fetch_add(1, Ordering::SeqCst);
        }
        if frame.frame_type != FrameType::Headers as u8 {
            continue;
        }
        write_raw_headers(
            &mut stream,
            frame.stream_id,
            &[
                (":status", "200"),
                ("content-encoding", "gzip"),
                ("content-length", "2000"),
            ],
            false,
        )
        .await;
        let mut body = vec![0x1f, 0x8b, 0x07, 0x00];
        body.resize(1000, b'x');
        write_data(&mut stream, frame.stream_id, &body, false).await;
    }
}

#[tokio::test]
async fn a_failed_decoded_h2_stream_that_is_kept_releases_its_stream_and_host_slot() {
    use futures_util::StreamExt;
    let resets = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&resets);
    let (cert, key) = tls_support::self_signed();
    let der = cert.to_der().unwrap();
    let port = tls_support::tls_server(cert, key, Arc::new(AtomicUsize::new(0)), move |stream| {
        not_gzip(stream, Arc::clone(&counter))
    })
    .await;
    let session = Session::builder()
        .tls_trust(
            TlsTrustConfig::new()
                .env_roots(false)
                .system_roots(false)
                .add_ca_der(der),
        )
        .host_limits(leyline::HostLimits::new().max_in_flight(1))
        .build()
        .unwrap();
    let mut body = session
        .get(format!("https://127.0.0.1:{port}/"))
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
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while resets.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the stream is reset while the failed body is kept");
    drop(body);
}
