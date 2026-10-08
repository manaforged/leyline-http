#[path = "h2_support/mod.rs"]
mod support;

use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use leyline::h2::config::{H2Config, PseudoOrder, SettingId};
use leyline::h2::connection::PseudoHeaders;
use leyline::h2::error::ErrorCode;
use leyline::h2::frame::FrameType;
use leyline::h2::{Head, RequestBody};
use support::*;
use tokio::io::AsyncWriteExt;

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

fn encode_rst_stream(stream_id: u32, code: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(13);
    v.extend_from_slice(&[0x00, 0x00, 0x04, 0x03, 0x00]);
    v.extend_from_slice(&stream_id.to_be_bytes());
    v[5] &= 0x7F;
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

        for expected_sid in [1u32, 3, 5, 7] {
            let (h, _) = read_frame(&mut server_io).await;
            assert_eq!(h.frame_type, FrameType::Headers as u8);
            assert_eq!(h.stream_id, expected_sid);
            server_io
                .write_all(&encode_rst_stream(expected_sid, 0x8))
                .await
                .expect("rst write");
        }

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
        drop(server_io.shutdown().await);
    });

    let handle = leyline::h2::start(client_io, flood_config())
        .await
        .expect("handshake");

    let mut tasks = Vec::new();
    for i in 0..4 {
        let handle = handle.clone();
        let (p, h) = get_req(&format!("/{i}"));
        tasks.push(tokio::spawn(async move {
            handle
                .send_shared(
                    Arc::new(Head {
                        pseudo: p,
                        headers: h,
                    }),
                    RequestBody::None,
                    false,
                )
                .await
        }));
    }

    let mut results = Vec::new();
    for t in tasks {
        let r = tokio::time::timeout(Duration::from_secs(3), t)
            .await
            .expect("request must finish on RST flood")
            .expect("request task");
        results.push(r);
    }
    let err = results
        .into_iter()
        .filter_map(Result::err)
        .find(|e| matches!(e, leyline::h2::H2Error::Connection { .. }))
        .expect("an in-flight request must carry the connection error");
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

    drop(server.await);
}
