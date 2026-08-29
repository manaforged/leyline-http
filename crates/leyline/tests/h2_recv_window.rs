//! Regression gate: the inbound receive window must be *enforced*, not
//! merely advertised.
//!
//! `on_data` decrements `conn_recv_window` and the stream's `recv_window`
//! per DATA frame but, before the fix, never checked for overrun — the
//! windows silently went negative and were topped back up. RFC 9113
//! §6.9.1 permits treating an over-window DATA frame as FLOW_CONTROL_ERROR.
//! A compliant peer never drives a receive window below zero, so an
//! over-window burst is a connection-level violation (whole connection
//! torn down) or, when only a single stream overruns, a stream-level one
//! (RST_STREAM, connection survives).
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

/// Connection window and stream window both default (65535). A large
/// advertised `max_frame_size` lets the mock server put an over-window
/// DATA frame on the wire in one shot.
fn test_config() -> H2Config {
    H2Config {
        settings: vec![
            (SettingId::HeaderTableSize, 4096),
            (SettingId::EnablePush, 0),
            (SettingId::InitialWindowSize, 65535),
            (SettingId::MaxFrameSize, 1_048_576),
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
        header_block_reassembly_timeout: Duration::from_secs(10),
    }
}

/// Like `test_config` but with a *small* per-stream window (1024) while
/// the connection window stays at 65535. This isolates the stream-level
/// guard: a mid-sized DATA frame overruns the stream window without
/// touching the connection window.
fn small_stream_window_config() -> H2Config {
    let mut cfg = test_config();
    cfg.settings = vec![
        (SettingId::HeaderTableSize, 4096),
        (SettingId::EnablePush, 0),
        (SettingId::InitialWindowSize, 1024),
        (SettingId::MaxFrameSize, 1_048_576),
    ];
    cfg
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

/// A single DATA frame larger than the advertised *connection* window
/// (65535) — plus the small slack — must tear the whole connection down
/// with FLOW_CONTROL_ERROR rather than letting the window silently go
/// negative.
#[tokio::test]
async fn connection_recv_window_overrun_kills_connection() {
    let (client_io, mut server_io) = tokio::io::duplex(256 * 1024);

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        // Our config advertises a 1 MiB MAX_FRAME_SIZE (below), so the reader
        // accepts the single large DATA frame that overruns the window. The
        // server's own MAX_FRAME_SIZE here only bounds what we send, so it is
        // irrelevant to this inbound test.
        write_server_settings_with(&mut server_io, vec![(0x5, 1_048_576)]).await;
        write_settings_ack(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert!(h.flags & 0x1 != 0, "expected client SETTINGS ack");

        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Headers as u8);
        assert_eq!(h.stream_id, 1);

        // 100_000 bytes > 65535 conn window + slack → connection error. Our
        // reader accepts this single large frame because our config advertises
        // a 1 MiB MAX_FRAME_SIZE.
        write_data(&mut server_io, 1, &vec![0u8; 100_000], false).await;

        // Drain until the client tears the connection down (EOF).
        let mut sink = BytesMut::with_capacity(4096);
        let mut buf = [0u8; 256];
        while let Ok(n) = server_io.read(&mut buf).await {
            if n == 0 {
                break;
            }
            sink.extend_from_slice(&buf[..n]);
        }
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
        .expect("driver must terminate on a connection-window overrun")
        .expect_err("driver must report an error");
    match err {
        leyline::h2::H2Error::Connection { code, reason } => {
            assert_eq!(code, ErrorCode::FlowControlError, "wrong code: {reason}");
            assert!(
                reason.contains("connection receive window"),
                "unexpected reason: {reason}"
            );
        }
        other => panic!("expected Connection error, got {other:?}"),
    }

    let _ = req_fut.await;
    let _ = server.await;
}

/// A DATA frame that overruns only the *stream* window (1024) while the
/// connection window (65535) has headroom must RST_STREAM the offending
/// stream with FLOW_CONTROL_ERROR and leave the connection intact.
#[tokio::test]
async fn stream_recv_window_overrun_rsts_stream_and_survives() {
    let (client_io, mut server_io) = tokio::io::duplex(256 * 1024);

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        write_server_settings_with(&mut server_io, vec![(0x5, 1_048_576)]).await;
        write_settings_ack(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert!(h.flags & 0x1 != 0, "expected client SETTINGS ack");

        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Headers as u8);
        assert_eq!(h.stream_id, 1);

        // Response HEADERS first, so the DATA frame is valid per the
        // stream state machine and reaches the flow-control accounting.
        write_response_headers(&mut server_io, 1).await;
        // 40_000 bytes > 1024 stream window + slack, but < 65535 conn window →
        // stream-level violation only. Our 1 MiB advertised MAX_FRAME_SIZE lets
        // the reader accept this single large frame.
        write_data(&mut server_io, 1, &vec![0u8; 40_000], false).await;

        // The client must RST_STREAM(FLOW_CONTROL_ERROR) on stream 1 while
        // keeping the connection alive.
        let (h, payload) = read_frame(&mut server_io).await;
        assert_eq!(
            h.frame_type,
            FrameType::RstStream as u8,
            "expected RST_STREAM on the offending stream"
        );
        assert_eq!(h.stream_id, 1);
        let code = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
        assert_eq!(
            code,
            ErrorCode::FlowControlError as u32,
            "RST_STREAM must carry FLOW_CONTROL_ERROR"
        );

        // Drain the rest (conn-window WINDOW_UPDATE, then GOAWAY + EOF once
        // the handle is dropped).
        let mut buf = [0u8; 256];
        while let Ok(n) = server_io.read(&mut buf).await {
            if n == 0 {
                break;
            }
        }
    });

    let (handle, driver) = ClientConnection::start(client_io, small_stream_window_config())
        .await
        .expect("handshake");

    let (p, h) = get_req("/");
    let req_result = handle.send_request(p, h, None).await;

    // The stream fails with FLOW_CONTROL_ERROR...
    match req_result {
        Err(leyline::h2::H2Error::Stream { stream_id, code }) => {
            assert_eq!(stream_id, 1);
            assert_eq!(code, ErrorCode::FlowControlError);
        }
        other => panic!("expected a stream FlowControlError, got {other:?}"),
    }

    // ...but the connection survives: dropping the handle drives a graceful
    // shutdown, so the driver exits cleanly rather than with a connection
    // error.
    drop(handle);
    let driver_result = tokio::time::timeout(Duration::from_secs(2), driver.join()).await;
    let outcome = driver_result.expect("driver must finish after graceful shutdown");
    assert!(
        outcome.is_ok(),
        "connection must survive a stream-level flow violation, got {outcome:?}"
    );

    let _ = server.await;
}
