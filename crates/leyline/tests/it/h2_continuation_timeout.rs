use crate::h2_support as support;

use std::sync::Arc;
use std::time::Duration;

use leyline::internals::h2::H2Config;
use leyline::internals::h2::H2Limits;
use leyline::internals::h2::frame::FrameType;
use support::*;

const REASSEMBLY_TIMEOUT: Duration = Duration::from_millis(250);

fn test_config() -> H2Config {
    H2Config {
        limits: H2Limits {
            settings_flood_threshold: 100,
            header_block_reassembly_timeout: REASSEMBLY_TIMEOUT,
            ..H2Limits::default()
        },
        ..H2Config::default()
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

    let handle = leyline::internals::h2::start(client_io, test_config())
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

#[tokio::test]
async fn a_reassembly_timeout_of_duration_max_sets_no_deadline() {
    let (client_io, mut server_io) = tokio::io::duplex(65_536);

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        write_server_settings(&mut server_io).await;
        write_settings_ack(&mut server_io).await;
        loop {
            let (h, _) = read_frame(&mut server_io).await;
            if h.frame_type == FrameType::Headers as u8 {
                write_headers_without_end(&mut server_io, h.stream_id).await;
                write_end_headers(&mut server_io, h.stream_id).await;
                write_data(&mut server_io, h.stream_id, b"ok", true).await;
            }
        }
    });

    let mut config = test_config();
    config.limits.header_block_reassembly_timeout = Duration::MAX;
    let handle = leyline::internals::h2::start(client_io, config)
        .await
        .expect("handshake");
    let sent = handle.send_shared(Arc::new(get_head("/")), leyline::Body::default(), false);
    let response = tokio::time::timeout(Duration::from_secs(5), sent)
        .await
        .expect("the response arrives")
        .expect("Duration::MAX sets no reassembly deadline");
    assert_eq!(response.status, 200);

    server.abort();
}
