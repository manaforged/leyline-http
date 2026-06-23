//! Regression gate for RFC 9113 §6.9.1 flow-control window
//! overflow. A peer that pushes the send-side window past 2^31 − 1
//! via `WINDOW_UPDATE` must be rejected with `FLOW_CONTROL_ERROR`
//! rather than silently accumulating into the `i64` counter.
//!
//! The bug shape: `self.conn_send_window += w.increment
//! as i64` with no clamp. A malicious server sending
//! `WINDOW_UPDATE(stream=0, inc=0x7FFFFFFF)` twice would climb past
//! 2^31 − 1 without any error surfaced to the caller — an interop
//! + compliance failure (h2spec 6.9.1).

#[path = "h2_support/mod.rs"]
mod support;

use std::time::Duration;

use bytes::BytesMut;
use leyline::h2::config::{H2Config, PseudoOrder, SettingId};
use leyline::h2::connection::{ClientConnection, PseudoHeaders};
use leyline::h2::error::ErrorCode;
use leyline::h2::frame::FrameType;
use support::*;
use tokio::io::AsyncReadExt;

fn test_config() -> H2Config {
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
        rst_stream_flood_threshold: 100,
        rst_stream_flood_window: Duration::from_secs(10),
        settings_ack_timeout: Duration::from_secs(10),
        max_response_body_bytes: 100 * 1024 * 1024,
        max_header_block_bytes: 256 * 1024,
        settings_flood_threshold: 100,
        settings_flood_window: Duration::from_secs(10),
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

#[tokio::test]
async fn connection_window_update_overflow_kills_connection() {
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

        // Initial conn window = 65535. Push it up by 0x7FFFFFFF twice
        // and the client's accumulator goes past 2^31-1. The client
        // MUST surface FlowControlError rather than silently climbing.
        //
        // First WU bumps the window to 65535 + 2147483647 = well past
        // the 2^31-1 cap, so one is enough to trip the guard.
        write_window_update(&mut server_io, 0, 0x7FFF_FFFF).await;

        // Drain whatever the client sends back (expect GOAWAY).
        let mut sink = BytesMut::with_capacity(2048);
        let mut buf = [0u8; 256];
        while let Ok(n) = server_io.read(&mut buf).await {
            if n == 0 {
                break;
            }
            sink.extend_from_slice(&buf[..n]);
            if sink.len() >= 17 {
                break;
            }
        }
    });

    let (handle, driver) = ClientConnection::start(client_io, test_config())
        .await
        .expect("handshake");

    // Fire a request; the overflow should surface either on the
    // pending request or on the driver shutdown.
    let (p, h) = get_req("/");
    let req_fut = tokio::spawn({
        let handle = handle.clone();
        async move { handle.send_request(p, h, None).await }
    });

    let driver_result = tokio::time::timeout(Duration::from_secs(2), driver.join()).await;

    // The driver must exit with a connection error carrying
    // FlowControlError. It MUST NOT silently keep running.
    let err = driver_result
        .expect("driver must terminate on overflow")
        .expect_err("driver must report error");
    match err {
        leyline::h2::H2Error::Connection { code, .. } => {
            assert_eq!(code, ErrorCode::FlowControlError, "wrong error code");
        }
        other => panic!("expected Connection error, got {other:?}"),
    }

    // Request also surfaces as failed (either flow-control error or
    // aborted with the driver).
    let _ = req_fut.await;
    let _ = server.await;
}

/// The same guard covers SETTINGS_INITIAL_WINDOW_SIZE overflow per
/// RFC 9113 §6.9.2. The integration path differs from WINDOW_UPDATE:
/// the delta is computed from `new_initial - old_initial` and
/// applied to every active stream. This test opens a stream, bumps
/// its send window, then has the peer set INITIAL_WINDOW_SIZE to the
/// RFC max (2^31 − 1). The cumulative delta pushes the stream
/// window past the cap → the driver must emit GOAWAY(FlowControlError).
#[tokio::test]
async fn settings_initial_window_overflow_kills_connection() {
    use leyline::h2::frame::FrameType;
    use tokio::io::AsyncWriteExt;

    let (client_io, mut server_io) = tokio::io::duplex(65_536);

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        // Server SETTINGS with initial_window_size = 65535 (default).
        write_server_settings(&mut server_io).await;
        // Server ACKs client settings.
        write_settings_ack(&mut server_io).await;
        // Client ACKs server settings.
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        assert!(h.flags & 0x1 != 0);

        // Wait for HEADERS on stream 1 so the stream exists in the
        // driver's table BEFORE we ship the malicious SETTINGS.
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Headers as u8);
        assert_eq!(h.stream_id, 1);

        // Bump stream 1 send window by 0x7FFFFFFF via WINDOW_UPDATE.
        // Now stream 1 send_window = 65535 + 0x7FFFFFFF = slightly
        // over MAX_FLOW_WINDOW — guard already rejects, so use a
        // safe delta that keeps it below cap but leaves no headroom.
        // Pick inc = MAX - 65535 - 1 so send_window = MAX - 1.
        let inc: u32 = 0x7FFF_FFFE - 65_535;
        write_window_update(&mut server_io, 1, inc).await;

        // Now SETTINGS(initial_window_size = 2^31-1). delta =
        // 2^31-1 - 65535. Applied to stream 1: new = (MAX-1) + (MAX-65535)
        // which massively exceeds MAX → FlowControlError.
        write_server_settings_with(&mut server_io, vec![(0x4, 0x7FFF_FFFF)]).await;

        // Drain any GOAWAY the client writes before FIN.
        let mut sink = BytesMut::with_capacity(4096);
        let mut buf = [0u8; 256];
        while let Ok(n) = server_io.read(&mut buf).await {
            if n == 0 {
                break;
            }
            sink.extend_from_slice(&buf[..n]);
            if sink.len() >= 17 {
                break;
            }
        }
        let _ = server_io.shutdown().await;
    });

    let (handle, driver) = ClientConnection::start(client_io, test_config())
        .await
        .expect("handshake");

    let (p, h) = get_req("/");
    let req_fut = tokio::spawn({
        let handle = handle.clone();
        async move { handle.send_request(p, h, None).await }
    });

    let driver_result = tokio::time::timeout(Duration::from_secs(2), driver.join()).await;
    let err = driver_result
        .expect("driver must terminate on SETTINGS overflow")
        .expect_err("driver must report error");
    match err {
        leyline::h2::H2Error::Connection { code, reason } => {
            assert_eq!(code, ErrorCode::FlowControlError, "wrong code: {reason}");
            assert!(
                reason.contains("SETTINGS_INITIAL_WINDOW_SIZE"),
                "unexpected reason: {reason}"
            );
        }
        other => panic!("expected Connection error, got {other:?}"),
    }

    let _ = req_fut.await;
    let _ = server.await;
}
