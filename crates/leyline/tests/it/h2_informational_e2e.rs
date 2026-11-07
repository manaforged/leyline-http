use crate::h2_support as support;

use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use leyline::internals::h2::H2Config;
use leyline::internals::h2::H2Limits;
use leyline::internals::h2::Head;
use leyline::internals::h2::connection::PseudoHeaders;
use leyline::internals::h2::frame::{DataFrame, FrameType, HeadersFrame};
use leyline::internals::h2::hpack;
use support::*;
use tokio::io::{AsyncWrite, AsyncWriteExt};

const END_STREAM: u8 = 0x1;

fn config() -> H2Config {
    H2Config {
        limits: H2Limits {
            settings_flood_threshold: 100,
            ..H2Limits::default()
        },
        ..H2Config::default()
    }
}

async fn write_headers_block<S: AsyncWrite + Unpin>(
    s: &mut S,
    enc: &mut hpack::Encoder,
    stream_id: u32,
    headers: &[(&str, &str)],
    end_stream: bool,
) {
    let fragment = enc.encode_header_block(headers);
    let h = HeadersFrame {
        stream_id,
        end_stream,
        end_headers: true,
        priority: None,
        fragment: bytes::Bytes::from(fragment),
    };
    let mut buf = BytesMut::new();
    h.encode(&mut buf);
    s.write_all(&buf).await.expect("headers write");
}

#[tokio::test]
async fn early_hints_103_is_skipped_final_status_wins() {
    let (client_io, mut server_io) = tokio::io::duplex(1 << 20);

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8, "client SETTINGS");
        write_server_settings(&mut server_io).await;
        write_settings_ack(&mut server_io).await;

        let stream_id = loop {
            let (h, _) = read_frame(&mut server_io).await;
            if h.frame_type == FrameType::Headers as u8 {
                assert_ne!(h.flags & END_STREAM, 0, "no-body GET carries END_STREAM");
                break h.stream_id;
            }
        };

        let mut enc = hpack::Encoder::new();
        write_headers_block(
            &mut server_io,
            &mut enc,
            stream_id,
            &[(":status", "103"), ("link", "</style.css>; rel=preload")],
            false,
        )
        .await;
        write_headers_block(
            &mut server_io,
            &mut enc,
            stream_id,
            &[(":status", "200"), ("content-type", "text/html")],
            false,
        )
        .await;
        let d = DataFrame {
            stream_id,
            end_stream: true,
            data: bytes::Bytes::from_static(b"<html>ok</html>"),
            wire_len: 15,
        };
        let mut buf = BytesMut::new();
        d.encode(&mut buf);
        server_io.write_all(&buf).await.expect("data write");
        drop(server_io.shutdown().await);
    });

    let handle = leyline::internals::h2::start(client_io, config())
        .await
        .expect("handshake");

    let resp = tokio::time::timeout(
        Duration::from_secs(5),
        handle.send_shared(
            Arc::new(Head::new(
                PseudoHeaders {
                    method: "GET".into(),
                    scheme: "https".into(),
                    authority: "example.com".into(),
                    path: "/".into(),
                    protocol: None,
                },
                vec![],
            )),
            leyline::Body::default(),
            false,
        ),
    )
    .await
    .expect("request timed out")
    .expect("a 103 before the 200 must not error the request");

    assert_eq!(
        resp.status, 200,
        "1xx Early Hints must be skipped, not returned as the final status"
    );
    let leyline::internals::pool::TransportBody::Buffered(body) = resp.body else {
        panic!("buffered body")
    };
    assert_eq!(body, b"<html>ok</html>");
    assert!(
        resp.headers
            .iter()
            .any(|(k, v)| k == "content-type" && v == "text/html"),
        "final headers must come from the 200, got {:?}",
        resp.headers
    );
    assert!(
        !resp.headers.iter().any(|(k, _)| k == "link"),
        "103 Early Hints headers must be discarded, got {:?}",
        resp.headers
    );

    server.await.expect("mock server panicked");
}

#[tokio::test]
async fn endless_early_hints_fail_the_request() {
    let (client_io, mut server_io) = tokio::io::duplex(1 << 20);

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        read_frame(&mut server_io).await;
        write_server_settings(&mut server_io).await;
        write_settings_ack(&mut server_io).await;
        let stream_id = loop {
            let (h, _) = read_frame(&mut server_io).await;
            if h.frame_type == FrameType::Headers as u8 {
                break h.stream_id;
            }
        };
        let mut enc = hpack::Encoder::new();
        for _ in 0..64 {
            write_headers_block(
                &mut server_io,
                &mut enc,
                stream_id,
                &[(":status", "103")],
                false,
            )
            .await;
        }
        write_headers_block(
            &mut server_io,
            &mut enc,
            stream_id,
            &[(":status", "200")],
            true,
        )
        .await;
        server_io
    });

    let handle = leyline::internals::h2::start(client_io, config())
        .await
        .expect("handshake");
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        handle.send_shared(
            Arc::new(Head::new(
                PseudoHeaders {
                    method: "GET".into(),
                    scheme: "https".into(),
                    authority: "example.com".into(),
                    path: "/".into(),
                    protocol: None,
                },
                vec![],
            )),
            leyline::Body::default(),
            false,
        ),
    )
    .await
    .expect("request timed out");
    let Err(_) = result else {
        panic!("64 informational responses must fail the stream");
    };
    drop(server.await.expect("mock server panicked"));
}
