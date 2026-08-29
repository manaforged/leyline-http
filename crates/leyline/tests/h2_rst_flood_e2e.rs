//! End-to-end coverage gate for CVE-2023-44487
//! (Rapid Reset / RST_STREAM flood).
//!
//! `tests/rst_flood.rs` exercises the `RstFloodDetector` in pure
//! synthetic-timestamp mode. That proved the detector MATH, not
//! that the detector is actually wired into the frame-handling
//! path. This test drives a real `tokio::io::duplex` with a mock
//! server that emits `RST_STREAM` frames past the configured
//! threshold and asserts the driver surfaces
//! `ENHANCE_YOUR_CALM`. If a future refactor ever moves the
//! `rst_flood.record(..)` call out of the `Frame::RstStream` arm,
//! this test fails.
#[path = "h2_support/mod.rs"]
mod support;

use std::time::Duration;

use bytes::BytesMut;
use leyline::h2::config::{H2Config, PseudoOrder, SettingId};
use leyline::h2::connection::{ClientConnection, PseudoHeaders};
use leyline::h2::error::ErrorCode;
use leyline::h2::frame::FrameType;
use support::*;
use tokio::io::AsyncWriteExt;

/// Config with a deliberately low `rst_stream_flood_threshold` so
/// a handful of `RST_STREAM` frames is enough to trip the guard.
fn flood_config() -> H2Config {
    H2Config {
        settings: vec![
            (SettingId::HeaderTableSize, 4096),
            (SettingId::EnablePush, 0),
            (SettingId::InitialWindowSize, 65535),
            (SettingId::MaxFrameSize, 16384),
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
        initial_connection_window_size: 65535,
        default_priority: None,
        rst_stream_flood_threshold: 3,
        rst_stream_flood_window: Duration::from_secs(10),
        settings_ack_timeout: Duration::from_secs(10),
        max_response_body_bytes: 100 * 1024 * 1024,
        max_header_block_bytes: 256 * 1024,
        settings_flood_threshold: 100,
        settings_flood_window: Duration::from_secs(10),
        header_block_reassembly_timeout: Duration::from_secs(10),
    }
}

type CowHeaders = Vec<(
    std::borrow::Cow<'static, str>,
    std::borrow::Cow<'static, str>,
)>;

fn get_req(path: &str) -> (PseudoHeaders, CowHeaders) {
    (
        PseudoHeaders {
            method: "GET".into(),
            scheme: "https".into(),
            authority: "example.com".into(),
            path: path.into(),
            protocol: None,
        },
        vec![("user-agent".into(), "test".into())],
    )
}

/// Build a raw RST_STREAM frame (9-byte header + 4-byte error code
/// payload). The support helpers don't expose one — this is a
/// throwaway encoding helper for the flood test only.
fn encode_rst_stream(stream_id: u32, code: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(13);
    // 9-byte frame header: length=4, type=RST_STREAM (0x3), flags=0,
    // stream id (31-bit, MSB-first; top bit reserved/zero).
    v.extend_from_slice(&[0x00, 0x00, 0x04, 0x03, 0x00]);
    v.extend_from_slice(&stream_id.to_be_bytes());
    v[5] &= 0x7F; // clear reserved bit per RFC 9113 §4.1
    // 4-byte payload: error code.
    v.extend_from_slice(&code.to_be_bytes());
    v
}

#[tokio::test]
async fn rst_stream_flood_trips_enhance_your_calm() {
    let (client_io, mut server_io) = tokio::io::duplex(65_536);

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        write_server_settings(&mut server_io).await;
        write_settings_ack(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        assert!(h.flags & 0x1 != 0);

        // Accept four client HEADERS (streams 1, 3, 5, 7) and emit
        // RST_STREAM(CANCEL) on each. Threshold is 3, so the 4th
        // triggers ENHANCE_YOUR_CALM before the stream error even
        // reaches the caller.
        for expected_sid in [1u32, 3, 5, 7] {
            let (h, _) = read_frame(&mut server_io).await;
            assert_eq!(h.frame_type, FrameType::Headers as u8);
            assert_eq!(h.stream_id, expected_sid);
            // Write a RST_STREAM(CANCEL=0x8) on this stream.
            server_io
                .write_all(&encode_rst_stream(expected_sid, 0x8))
                .await
                .expect("rst write");
        }

        // Drain the GOAWAY the driver must send before FIN.
        let mut sink = BytesMut::with_capacity(4096);
        let mut buf = [0u8; 256];
        loop {
            match tokio::time::timeout(Duration::from_secs(1), async {
                tokio::io::AsyncReadExt::read(&mut server_io, &mut buf).await
            })
            .await
            {
                Ok(Ok(0)) | Err(_) => break,
                Ok(Ok(n)) => sink.extend_from_slice(&buf[..n]),
                Ok(Err(_)) => break,
            }
        }
        let _ = server_io.shutdown().await;
    });

    let (handle, driver) = ClientConnection::start(client_io, flood_config())
        .await
        .expect("handshake");

    // Fire four concurrent requests so the driver opens four
    // streams before the first RST_STREAM arrives.
    let mut tasks = Vec::new();
    for i in 0..4 {
        let handle = handle.clone();
        let (p, h) = get_req(&format!("/{i}"));
        tasks.push(tokio::spawn(async move {
            let _ = handle.send_request(p, h, None).await;
        }));
    }

    let driver_result = tokio::time::timeout(Duration::from_secs(3), driver.join()).await;
    let err = driver_result
        .expect("driver must terminate on RST flood")
        .expect_err("driver must report error");
    match err {
        leyline::h2::H2Error::Connection { code, .. } => {
            assert_eq!(
                code,
                ErrorCode::EnhanceYourCalm,
                "RST flood must trip ENHANCE_YOUR_CALM"
            );
        }
        other => panic!("expected Connection error, got {other:?}"),
    }

    // Tear down the in-flight requests.
    for t in tasks {
        let _ = t.await;
    }
    let _ = server.await;
}
