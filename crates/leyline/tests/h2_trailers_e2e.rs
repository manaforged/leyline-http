//! Regression gate for oversized trailer blocks.

#[path = "h2_support/mod.rs"]
mod support;

use std::time::Duration;

use leyline::h2::config::{H2Config, PseudoOrder, SettingId};
use leyline::h2::connection::{ClientConnection, PseudoHeaders};
use leyline::h2::frame::FrameType;
use support::*;
use tokio::io::AsyncWriteExt;

const END_STREAM: u8 = 0x1;
const END_HEADERS: u8 = 0x4;

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
        max_response_body_bytes: 100 * 1024 * 1024,
        max_header_block_bytes: 256 * 1024,
        settings_flood_threshold: 100,
        settings_flood_window: Duration::from_secs(10),
        header_block_reassembly_timeout: Duration::from_secs(10),
    }
}

#[tokio::test]
async fn oversized_trailer_block_splits_into_continuations() {
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
                assert_eq!(h.flags & END_STREAM, 0, "request HEADERS carries trailers");
                assert_ne!(h.flags & END_HEADERS, 0, "request headers fit one frame");
                break h.stream_id;
            }
        };

        let (h, first) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Headers as u8, "trailer HEADERS");
        assert_ne!(
            h.flags & END_STREAM,
            0,
            "END_STREAM rides the first trailer frame"
        );
        assert_eq!(
            h.flags & END_HEADERS,
            0,
            "oversized trailer block must NOT claim END_HEADERS on the first frame"
        );
        let mut frames = 1usize;
        let mut total = first.len();
        loop {
            let (h, payload) = read_frame(&mut server_io).await;
            assert_eq!(
                h.frame_type, 0x9,
                "expected CONTINUATION, got {}",
                h.frame_type
            );
            assert!(payload.len() <= 16_384, "frame exceeds max_frame_size");
            frames += 1;
            total += payload.len();
            if h.flags & END_HEADERS != 0 {
                break;
            }
        }
        assert!(frames >= 2, "block must span multiple frames, got {frames}");
        assert!(
            total > 16_384,
            "reassembled block must exceed one frame: {total}"
        );

        write_response(&mut server_io, stream_id, b"ok").await;
        let _ = server_io.shutdown().await;
    });

    let (handle, _driver) = ClientConnection::start(client_io, config())
        .await
        .expect("handshake");

    let big = "a".repeat(60_000);
    let resp = tokio::time::timeout(
        Duration::from_secs(5),
        handle.send_request_with_trailers(
            PseudoHeaders {
                method: "POST".into(),
                scheme: "https".into(),
                authority: "example.com".into(),
                path: "/upload".into(),
                protocol: None,
            },
            vec![("user-agent".into(), "test".into())],
            None,
            vec![("x-blob-digest".into(), big)],
        ),
    )
    .await
    .expect("request timed out")
    .expect("oversized trailers must split into CONTINUATION frames, not error");

    assert_eq!(resp.status, 200);
    server.await.expect("mock server panicked");
}
