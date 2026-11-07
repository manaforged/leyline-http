use crate::h2_support as support;

use std::sync::Arc;
use std::time::Duration;

use support::*;
use tokio::io::AsyncReadExt;

use leyline::internals::h2::H2Config;
use leyline::internals::h2::H2Limits;
use leyline::internals::h2::Head;
use leyline::internals::h2::connection::PseudoHeaders;
use leyline::internals::h2::frame::FrameType;

fn test_config() -> H2Config {
    H2Config {
        limits: H2Limits {
            settings_flood_threshold: 100,
            ..H2Limits::default()
        },
        ..H2Config::default()
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
        drop(server_io.read(&mut sink).await);
    });

    let handle = leyline::internals::h2::start(client_io, test_config())
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
        drop(
            handle_clone
                .send_shared(
                    Arc::new(Head::new(pseudo, vec![])),
                    leyline::Body::default(),
                    false,
                )
                .await,
        );
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
