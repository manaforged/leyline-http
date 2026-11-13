#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use crate::h2_support as support;
use crate::tls_support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use leyline::internals::h2::frame::FrameType;
use leyline::{Session, Tls};
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
    let mut received = 0;
    loop {
        let (frame, payload) = read_frame(&mut stream).await;
        if frame.frame_type == FrameType::Settings as u8 && frame.flags & SETTINGS_ACK == 0 {
            write_settings_ack(&mut stream).await;
        }
        if frame.frame_type == FrameType::Data as u8 {
            received += payload.len();
        }
        let ends = frame.frame_type == FrameType::Headers as u8
            || frame.frame_type == FrameType::Data as u8;
        if !ends || frame.flags & END_STREAM == 0 {
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
        let body = std::mem::take(&mut received).to_string();
        write_data(&mut stream, frame.stream_id, body.as_bytes(), true).await;
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
        .tls(Tls::new().env_roots(false).system_roots(false).ca_der(der))
        .retry(0)
        .build()
        .unwrap();
    let url = format!("https://127.0.0.1:{port}/");
    session
        .get(format!("{url}a"))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let resp = session
        .put(format!("{url}b"))
        .body("hello")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.version(), http::Version::HTTP_2);
    assert_eq!(resp.bytes().await.unwrap(), "5");
    assert_eq!(connections.load(Ordering::SeqCst), 3);
}

async fn serve_h1(mut stream: SslStream<TcpStream>) {
    use tokio::io::AsyncReadExt;
    let mut seen = Vec::new();
    let mut chunk = [0; 1024];
    loop {
        let Ok(read) = stream.read(&mut chunk).await else {
            return;
        };
        if read == 0 {
            return;
        }
        seen.extend_from_slice(&chunk[..read]);
        if seen.ends_with(b"\r\n\r\n") {
            seen.clear();
            let reply = b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nhi";
            stream.write_all(reply).await.unwrap();
        }
    }
}

#[tokio::test]
async fn forced_http1_requests_keep_their_connection_beside_an_h2_one() {
    let (cert, key) = tls_support::self_signed();
    let der = cert.to_der().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let alpn = |index: usize| {
        if index == 0 {
            tls_support::H2
        } else {
            tls_support::HTTP11
        }
    };
    let port = tls_support::tls_server_per_connection(
        cert,
        key,
        Arc::clone(&connections),
        alpn,
        |index, stream| async move {
            match index {
                0 => serve_h2(stream, None).await,
                _ => serve_h1(stream).await,
            }
        },
    )
    .await;
    let session = Session::builder()
        .tls(Tls::new().env_roots(false).system_roots(false).ca_der(der))
        .retry(0)
        .build()
        .unwrap();
    let url = format!("https://127.0.0.1:{port}/");
    let mut seen = Vec::new();
    for (path, forced) in [("a", false), ("b", true), ("c", true), ("d", false)] {
        let mut request = session.get(format!("{url}{path}"));
        if forced {
            request = request.protocol(leyline::Protocol::Http1);
        }
        let response = request.send().await.unwrap();
        seen.push((response.version(), response.reused()));
        response.bytes().await.unwrap();
    }
    assert_eq!(
        seen,
        [
            (http::Version::HTTP_2, false),
            (http::Version::HTTP_11, false),
            (http::Version::HTTP_11, true),
            (http::Version::HTTP_2, true),
        ]
    );
    assert_eq!(connections.load(Ordering::SeqCst), 2);
}
