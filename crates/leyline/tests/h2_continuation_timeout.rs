#[path = "h2_support/mod.rs"]
mod support;

use std::time::Duration;

use leyline::h2::config::{H2Config, PseudoOrder, SettingId};
use leyline::h2::frame::FrameType;
use support::*;

const REASSEMBLY_TIMEOUT: Duration = Duration::from_millis(250);

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
        header_block_reassembly_timeout: REASSEMBLY_TIMEOUT,
    }
}

#[tokio::test]
async fn continuation_reassembly_times_out_on_stall() {
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

        write_headers_without_end(&mut server_io, 1).await;
        std::future::pending::<()>().await;
    });

    let handle = leyline::h2::start(client_io, test_config())
        .await
        .expect("handshake");
    tokio::time::timeout(Duration::from_secs(10), async {
        while !handle.is_closed() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("connection must close on a CONTINUATION stall, not hang");

    server.abort();
}
