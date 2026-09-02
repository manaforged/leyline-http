//! WINDOW_UPDATE past 2^31-1 is FLOW_CONTROL_ERROR.

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

        write_window_update(&mut server_io, 0, 0x7FFF_FFFF).await;

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

    let (p, h) = get_req("/");
    let req_fut = tokio::spawn({
        let handle = handle.clone();
        async move { handle.send_request(p, h, None).await }
    });

    let driver_result = tokio::time::timeout(Duration::from_secs(2), driver.join()).await;

    let err = driver_result
        .expect("driver must terminate on overflow")
        .expect_err("driver must report error");
    match err {
        leyline::h2::H2Error::Connection { code, .. } => {
            assert_eq!(code, ErrorCode::FlowControlError, "wrong error code");
        }
        other => panic!("expected Connection error, got {other:?}"),
    }

    let _ = req_fut.await;
    let _ = server.await;
}

#[tokio::test]
async fn settings_initial_window_overflow_kills_connection() {
    use leyline::h2::frame::FrameType;
    use tokio::io::AsyncWriteExt;

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

        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Headers as u8);
        assert_eq!(h.stream_id, 1);

        let inc: u32 = 0x7FFF_FFFE - 65_535;
        write_window_update(&mut server_io, 1, inc).await;

        write_server_settings_with(&mut server_io, vec![(0x4, 0x7FFF_FFFF)]).await;

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
