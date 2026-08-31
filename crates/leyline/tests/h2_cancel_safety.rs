//! Dropping send_request RST_STREAMs the H2 stream.

#[path = "h2_support/mod.rs"]
mod support;

use std::time::Duration;

use support::*;
use tokio::io::AsyncReadExt;

use leyline::h2::config::{H2Config, PseudoOrder, SettingId};
use leyline::h2::connection::{ClientConnection, PseudoHeaders};
use leyline::h2::frame::FrameType;

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
        settings_flood_window: std::time::Duration::from_secs(10),
        header_block_reassembly_timeout: std::time::Duration::from_secs(10),
    }
}

#[tokio::test]
async fn cancelled_send_request_rst_streams_the_slot() {
    let (client_io, mut server_io) = tokio::io::duplex(65_536);

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        let (_h, _) = read_frame(&mut server_io).await;
        write_server_settings(&mut server_io).await;
        write_settings_ack(&mut server_io).await;
        let (_h, _) = read_frame(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Headers as u8);
        let request_stream_id = h.stream_id;

        let started = std::time::Instant::now();
        let mut saw_rst = false;
        while started.elapsed() < Duration::from_secs(3) {
            let read =
                tokio::time::timeout(Duration::from_millis(500), read_frame(&mut server_io)).await;
            match read {
                Ok((hdr, payload)) => {
                    if hdr.frame_type == FrameType::RstStream as u8
                        && hdr.stream_id == request_stream_id
                    {
                        let code =
                            u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                        assert_eq!(code, 0x8, "expected RST_STREAM(CANCEL)");
                        saw_rst = true;
                        break;
                    }
                }
                Err(_) => continue,
            }
        }
        assert!(
            saw_rst,
            "driver should RST_STREAM(CANCEL) within 3s of the caller dropping the future"
        );

        let mut sink = [0u8; 256];
        let _ = server_io.read(&mut sink).await;
    });

    let (handle, _driver) = ClientConnection::start(client_io, test_config())
        .await
        .expect("handshake");

    let pseudo = PseudoHeaders {
        method: "GET".into(),
        scheme: "https".into(),
        authority: "example.com".into(),
        path: "/".into(),
        protocol: None,
    };

    let handle_clone = handle.clone();
    let pending = tokio::spawn(async move {
        let _ = handle_clone.send_request(pseudo, vec![], None).await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    pending.abort();

    tokio::time::sleep(Duration::from_millis(500)).await;

    drop(handle);

    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("server task should finish within 5s")
        .expect("server task panicked");
}
