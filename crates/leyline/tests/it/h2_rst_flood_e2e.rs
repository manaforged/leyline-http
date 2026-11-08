use crate::h2_support as support;

use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use leyline::internals::h2::H2Config;
use leyline::internals::h2::H2Limits;
use leyline::internals::h2::Head;
use leyline::internals::h2::connection::PseudoHeaders;
use leyline::internals::h2::error::ErrorCode;
use leyline::internals::h2::frame::FrameType;
use support::*;
use tokio::io::AsyncWriteExt;

fn flood_config() -> H2Config {
    H2Config {
        limits: H2Limits {
            rst_stream_flood_threshold: 3,
            settings_flood_threshold: 100,
            ..H2Limits::default()
        },
        ..H2Config::default()
    }
}

type CowHeaders = Vec<(
    std::borrow::Cow<'static, str>,
    leyline::internals::h2::HeaderStr,
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

    let handle = leyline::internals::h2::start(client_io, flood_config())
        .await
        .expect("handshake");

    let mut tasks = Vec::new();
    for i in 0..4 {
        let handle = handle.clone();
        let (p, h) = get_req(&format!("/{i}"));
        tasks.push(tokio::spawn(async move {
            handle
                .send_shared(Arc::new(Head::new(p, h)), leyline::Body::default(), false)
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
        .find(|e| matches!(e, leyline::internals::h2::H2Error::Connection { .. }))
        .expect("an in-flight request must carry the connection error");
    match err {
        leyline::internals::h2::H2Error::Connection { code, .. } => {
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
