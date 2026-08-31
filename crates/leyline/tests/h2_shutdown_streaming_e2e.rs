//! Regression gate for streaming uploads during graceful shutdown.
#[path = "h2_support/mod.rs"]
mod support;

use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use leyline::h2::config::{H2Config, PseudoOrder, SettingId};
use leyline::h2::connection::{ClientConnection, PseudoHeaders};
use leyline::h2::frame::FrameType;
use leyline::h2::{RequestBody, ResponseBody};
use support::*;
use tokio::io::AsyncWriteExt;

const END_STREAM: u8 = 0x1;
const CHUNKS: usize = 4;
const CHUNK_LEN: usize = 8 * 1024;

fn config() -> H2Config {
    H2Config {
        settings: vec![
            (SettingId::HeaderTableSize, 4096),
            (SettingId::EnablePush, 0),
            (SettingId::InitialWindowSize, 65_535),
            (SettingId::MaxFrameSize, 16_384),
        ],
        settings_order: vec![
            SettingId::HeaderTableSize,
            SettingId::EnablePush,
            SettingId::InitialWindowSize,
            SettingId::MaxFrameSize,
        ],
        pseudo_order: [
            PseudoOrder::Method,
            PseudoOrder::Authority,
            PseudoOrder::Scheme,
            PseudoOrder::Path,
        ],
        initial_connection_window_size: 65_535,
        default_priority: None,
        rst_stream_flood_threshold: 100,
        rst_stream_flood_window: Duration::from_secs(10),
        settings_ack_timeout: Duration::from_secs(10),
        max_response_body_bytes: 100 * 1024 * 1024,
        max_header_block_bytes: 256 * 1024,
        settings_flood_threshold: 100,
        settings_flood_window: Duration::from_secs(10),
        header_block_reassembly_timeout: Duration::from_secs(10),
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

        let mut enc = leyline::h2::hpack::Encoder::new();
        let fragment = enc.encode_header_block(&[(":status", "200")]);
        let hf = leyline::h2::frame::HeadersFrame {
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

        let done = leyline::h2::frame::DataFrame {
            stream_id,
            end_stream: true,
            data: Bytes::from_static(b"done"),
            wire_len: 4,
        };
        buf.clear();
        done.encode(&mut buf);
        server_io.write_all(&buf).await.expect("resp data");
        let _ = server_io.shutdown().await;
    });

    let (handle, driver) = ClientConnection::start(client_io, config())
        .await
        .expect("handshake");

    let chunks: Vec<std::io::Result<Bytes>> = (0..CHUNKS)
        .map(|i| Ok(Bytes::from(vec![i as u8 + 1; CHUNK_LEN])))
        .collect();
    let stream = futures_util::stream::iter(chunks).then(|c| async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        c
    });

    let resp = handle
        .send_request_ex(
            PseudoHeaders {
                method: "POST".into(),
                scheme: "https".into(),
                authority: "example.com".into(),
                path: "/upload".into(),
                protocol: None,
            },
            vec![("user-agent".into(), "test".into())],
            RequestBody::Streaming {
                stream: Box::pin(stream),
                length_hint: None,
            },
            true,
        )
        .await
        .expect("response headers");
    assert_eq!(resp.status, 200);

    drop(handle);

    let mut body = match resp.body {
        ResponseBody::Streaming(rx) => rx,
        other => panic!("expected streaming body, got {other:?}"),
    };
    let mut got = Vec::new();
    while let Some(chunk) = tokio::time::timeout(Duration::from_secs(5), body.recv())
        .await
        .expect("response stalled: upload never completed during shutdown")
    {
        got.extend_from_slice(&chunk.expect("body chunk"));
    }
    assert_eq!(got, b"done");

    tokio::time::timeout(Duration::from_secs(5), driver.join())
        .await
        .expect("driver hung")
        .expect("driver errored");
    server.await.expect("mock server panicked");
}
