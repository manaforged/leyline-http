use crate::h2_support as support;

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use leyline::internals::h2::H2Config;
use leyline::internals::h2::H2Limits;
use leyline::internals::h2::Head;
use leyline::internals::h2::connection::PseudoHeaders;
use leyline::internals::h2::frame::FrameType;
use support::*;
use tokio::io::AsyncWriteExt;
use tokio::time::{sleep, timeout};

const END_STREAM: u8 = 0x1;
const CHUNKS: usize = 4;
const CHUNK_LEN: usize = 8 * 1024;

fn config() -> H2Config {
    H2Config {
        limits: H2Limits {
            settings_flood_threshold: 100,
            ..H2Limits::default()
        },
        ..H2Config::default()
    }
}

#[tokio::test]
async fn streaming_upload_completes_after_last_handle_drops() {
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
                assert_eq!(h.flags & END_STREAM, 0, "streaming upload in flight");
                break h.stream_id;
            }
        };

        let mut enc = leyline::internals::h2::hpack::Encoder::new();
        let fragment = enc.encode_header_block(&[(":status", "200")]);
        let hf = leyline::internals::h2::frame::HeadersFrame {
            stream_id,
            end_stream: false,
            end_headers: true,
            priority: None,
            fragment: Bytes::from(fragment),
        };
        let mut buf = bytes::BytesMut::new();
        hf.encode(&mut buf);
        server_io.write_all(&buf).await.expect("resp headers");

        let mut received = 0usize;
        loop {
            let (h, payload) =
                tokio::time::timeout(Duration::from_secs(3), read_frame(&mut server_io))
                    .await
                    .expect("upload starved: no DATA during graceful shutdown");
            if h.frame_type != FrameType::Data as u8 {
                continue;
            }
            received += payload.len();
            if h.flags & END_STREAM != 0 {
                break;
            }
        }
        assert_eq!(received, CHUNKS * CHUNK_LEN, "full upload must arrive");

        let done = leyline::internals::h2::frame::DataFrame {
            stream_id,
            end_stream: true,
            data: Bytes::from_static(b"done"),
            wire_len: 4,
        };
        buf.clear();
        done.encode(&mut buf);
        server_io.write_all(&buf).await.expect("resp data");
        drop(server_io.shutdown().await);
    });

    let handle = leyline::internals::h2::start(client_io, config())
        .await
        .expect("handshake");

    let chunks: Vec<std::io::Result<Bytes>> = (0..CHUNKS)
        .map(|i| Ok(Bytes::from(vec![i as u8 + 1; CHUNK_LEN])))
        .collect();
    let stream = futures_util::stream::iter(chunks).then(|c| async move {
        sleep(Duration::from_millis(200)).await;
        c
    });

    let resp = handle
        .send_shared(
            Arc::new(Head::new(
                PseudoHeaders {
                    method: "POST".into(),
                    scheme: "https".into(),
                    authority: "example.com".into(),
                    path: "/upload".into(),
                    protocol: None,
                },
                vec![("user-agent".into(), "test".into())],
            )),
            leyline::Body::stream(stream, None),
            true,
        )
        .await
        .expect("response headers");
    assert_eq!(resp.status, 200);

    drop(handle);

    let mut body = match resp.body {
        leyline::internals::pool::TransportBody::Streaming(rx) => rx,
        other => panic!("expected streaming body, got {other:?}"),
    };
    let mut got = Vec::new();
    while let Some(chunk) = tokio::time::timeout(Duration::from_secs(5), body.next())
        .await
        .expect("response stalled: upload never completed during shutdown")
    {
        got.extend_from_slice(&chunk.expect("body chunk"));
    }
    assert_eq!(got, b"done");

    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("mock server hung")
        .expect("mock server panicked");
}

#[tokio::test]
async fn abandoned_response_is_reset_before_shutdown() {
    let (client, mut peer) = tokio::io::duplex(65_536);
    let server = tokio::spawn(async move {
        read_preface(&mut peer).await;
        let (header, _) = read_frame(&mut peer).await;
        assert_eq!(header.frame_type, FrameType::Settings as u8);
        write_server_settings(&mut peer).await;
        write_settings_ack(&mut peer).await;
        let sid = loop {
            let (header, _) = read_frame(&mut peer).await;
            if header.frame_type == FrameType::Headers as u8 {
                break header.stream_id;
            }
        };
        write_response_headers(&mut peer, sid).await;
        let mut reset = false;
        loop {
            let (header, payload) = timeout(Duration::from_secs(3), read_frame(&mut peer))
                .await
                .expect("connection shutdown");
            if header.frame_type == FrameType::RstStream as u8 {
                assert_eq!(header.stream_id, sid);
                assert_eq!(
                    u32::from_be_bytes(payload.try_into().expect("reset code")),
                    8
                );
                reset = true;
            } else if header.frame_type == FrameType::GoAway as u8 {
                assert!(reset, "abandoned response was not cancelled before GOAWAY");
                assert_eq!(
                    u32::from_be_bytes(payload[..4].try_into().expect("last peer stream")),
                    0
                );
                assert_eq!(
                    u32::from_be_bytes(payload[4..8].try_into().expect("GOAWAY code")),
                    0
                );
                break;
            }
        }
    });
    let handle = leyline::internals::h2::start(client, config())
        .await
        .expect("connection");
    let response = handle
        .send_shared(
            Arc::new(Head::new(
                PseudoHeaders {
                    method: "GET".into(),
                    scheme: "https".into(),
                    authority: "example.test".into(),
                    path: "/".into(),
                    protocol: None,
                },
                Vec::new(),
            )),
            leyline::Body::default(),
            true,
        )
        .await
        .expect("headers");
    assert_eq!(response.status, 200);
    drop(handle);
    drop(response);
    timeout(Duration::from_secs(3), server)
        .await
        .expect("peer sees cancellation and shutdown")
        .expect("peer task");
}
