#[path = "h2_support/mod.rs"]
mod support;

use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use leyline::h2::config::{H2Config, PseudoOrder, SettingId};
use leyline::h2::connection::PseudoHeaders;
use leyline::h2::frame::{DataFrame, FrameType, HeadersFrame};
use leyline::h2::hpack;
use leyline::h2::{Head, RequestBody};
use support::*;
use tokio::io::{AsyncWrite, AsyncWriteExt};

const END_STREAM: u8 = 0x1;

fn config() -> H2Config {
    H2Config {
        settings: vec![
            (SettingId::HeaderTableSize, 4096),
            (SettingId::EnablePush, 0),
            (SettingId::InitialWindowSize, 65_535),
            (SettingId::MaxFrameSize, 16_384),
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
        initial_connection_window_size: 65_535,
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

async fn write_headers_block<S: AsyncWrite + Unpin>(
    s: &mut S,
    enc: &mut hpack::Encoder,
    stream_id: u32,
    headers: &[(&str, &str)],
    end_stream: bool,
) {
    let fragment = enc.encode_header_block(headers);
    let h = HeadersFrame {
        stream_id,
        end_stream,
        end_headers: true,
        priority: None,
        fragment: bytes::Bytes::from(fragment),
    };
    let mut buf = BytesMut::new();
    h.encode(&mut buf);
    s.write_all(&buf).await.expect("headers write");
}

#[tokio::test]
async fn early_hints_103_is_skipped_final_status_wins() {
    let (client_io, mut server_io) = tokio::io::duplex(1 << 20);

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8, "client SETTINGS");
        write_server_settings(&mut server_io).await;
        write_settings_ack(&mut server_io).await;

        let stream_id = loop {
            let (h, _) = read_frame(&mut server_io).await;
            if h.frame_type == FrameType::Headers as u8 {
                assert_ne!(h.flags & END_STREAM, 0, "no-body GET carries END_STREAM");
                break h.stream_id;
            }
        };

        let mut enc = hpack::Encoder::new();
        write_headers_block(
            &mut server_io,
            &mut enc,
            stream_id,
            &[(":status", "103"), ("link", "</style.css>; rel=preload")],
            false,
        )
        .await;
        write_headers_block(
            &mut server_io,
            &mut enc,
            stream_id,
            &[(":status", "200"), ("content-type", "text/html")],
            false,
        )
        .await;
        let d = DataFrame {
            stream_id,
            end_stream: true,
            data: bytes::Bytes::from_static(b"<html>ok</html>"),
            wire_len: 15,
        };
        let mut buf = BytesMut::new();
        d.encode(&mut buf);
        server_io.write_all(&buf).await.expect("data write");
        let _ = server_io.shutdown().await;
    });

    let handle = leyline::h2::start(client_io, config())
        .await
        .expect("handshake");

    let resp = tokio::time::timeout(
        Duration::from_secs(5),
        handle.send_shared(
            Arc::new(Head {
                pseudo: PseudoHeaders {
                    method: "GET".into(),
                    scheme: "https".into(),
                    authority: "example.com".into(),
                    path: "/".into(),
                    protocol: None,
                },
                headers: vec![],
            }),
            RequestBody::None,
            false,
        ),
    )
    .await
    .expect("request timed out")
    .expect("a 103 before the 200 must not error the request");

    assert_eq!(
        resp.status, 200,
        "1xx Early Hints must be skipped, not returned as the final status"
    );
    let leyline::h2::ResponseBody::Buffered(body) = resp.body else {
        panic!("buffered body")
    };
    assert_eq!(body, b"<html>ok</html>");
    assert!(
        resp.headers
            .iter()
            .any(|(k, v)| k == "content-type" && v == "text/html"),
        "final headers must come from the 200, got {:?}",
        resp.headers
    );
    assert!(
        !resp.headers.iter().any(|(k, _)| k == "link"),
        "103 Early Hints headers must be discarded, got {:?}",
        resp.headers
    );

    server.await.expect("mock server panicked");
}
