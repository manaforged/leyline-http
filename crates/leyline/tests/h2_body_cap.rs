#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
#[path = "h2_support/mod.rs"]
mod support;
#[path = "tls_support/mod.rs"]
mod tls_support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use leyline::h2::frame::{FRAME_HEADER_LEN, FrameHeader, FrameType};
use leyline::{CompressionConfig, Kind, Session, TlsTrustConfig};
use leyline_bssl::pkey::{PKey, Private};
use leyline_bssl::ssl::{AlpnError, Ssl, SslContextBuilder, SslMethod, select_next_proto};
use leyline_bssl::x509::X509;
use support::{
    read_preface, write_data, write_response_headers, write_server_settings, write_settings_ack,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::net::TcpListener;

const CAP: usize = 1024;
const SMALL: usize = 16;
const LARGE: usize = 4096;
const SETTINGS_ACK: u8 = 0x1;

async fn next_frame<S: AsyncRead + Unpin>(stream: &mut S) -> Option<FrameHeader> {
    let mut head = [0u8; FRAME_HEADER_LEN];
    stream.read_exact(&mut head).await.ok()?;
    let header = FrameHeader::parse(&head);
    let mut payload = vec![0u8; header.length as usize];
    stream.read_exact(&mut payload).await.ok()?;
    Some(header)
}

async fn serve<S: AsyncRead + AsyncWrite + Unpin>(mut stream: S) {
    read_preface(&mut stream).await;
    write_server_settings(&mut stream).await;
    let mut answered = 0;
    while let Some(frame) = next_frame(&mut stream).await {
        if frame.frame_type == FrameType::Settings as u8 && frame.flags & SETTINGS_ACK == 0 {
            write_settings_ack(&mut stream).await;
        }
        if frame.frame_type != FrameType::Headers as u8 {
            continue;
        }
        let length = if answered == 0 { SMALL } else { LARGE };
        write_response_headers(&mut stream, frame.stream_id).await;
        write_data(&mut stream, frame.stream_id, &vec![b'x'; length], true).await;
        answered += 1;
    }
}

async fn h2_server(cert: X509, key: PKey<Private>, connections: Arc<AtomicUsize>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut context = SslContextBuilder::new(SslMethod::tls()).unwrap();
    context.set_certificate(&cert).unwrap();
    context.set_private_key(&key).unwrap();
    context.set_alpn_select_callback(|_, offered| {
        select_next_proto(b"\x02h2", offered).ok_or(AlpnError::NOACK)
    });
    let context = context.build();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            connections.fetch_add(1, Ordering::SeqCst);
            let ssl = Ssl::new(&context).unwrap();
            if let Ok(stream) = leyline_bssl_tokio::SslStreamBuilder::new(ssl, tcp)
                .accept()
                .await
            {
                tokio::spawn(serve(stream));
            }
        }
    });
    port
}

#[tokio::test]
async fn an_h2_body_over_the_cap_fails_on_the_kept_connection() {
    let (cert, key) = tls_support::self_signed();
    let der = cert.to_der().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let port = h2_server(cert, key, Arc::clone(&connections)).await;
    let session = Session::builder()
        .compression(CompressionConfig::new().max_body_size(CAP))
        .tls_trust(
            TlsTrustConfig::new()
                .env_roots(false)
                .system_roots(false)
                .add_ca_der(der),
        )
        .build()
        .unwrap();
    let url = format!("https://127.0.0.1:{port}/");
    let first = session.get(&url).await.unwrap().bytes().await.unwrap();
    assert_eq!(first.len(), SMALL);
    let second = async { session.get(&url).await?.bytes().await }.await;
    let err = second.unwrap_err();
    assert_eq!(err.kind(), Kind::Body, "{err:?}");
    assert_eq!(connections.load(Ordering::SeqCst), 1);
}
