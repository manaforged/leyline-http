//! Regression: HTTP/2 1xx informational responses (Cloudflare's 103 Early
//! Hints) must be skipped, not returned as the final status.
//!
//! The bug: the H2 client set `got_headers` on the FIRST HEADERS frame
//! unconditionally, so a 103 Early Hints became the response status and the
//! real 200 was mis-filed as trailers — surfacing in a consuming application
//! as an opaque "unexpected 103" error on every Cloudflare-fronted HTML fetch.
//! H1 already skipped 1xx (pool/h1.rs:571); the H2 client did not.
//!
//! The request is a no-body GET (END_STREAM on the request HEADERS → the stream
//! is HalfClosedLocal — the production path), and the two response header blocks
//! share ONE encoder so HPACK dynamic-table state stays continuous: a fix that
//! wrongly reset the decoder between the 103 and 200 would corrupt the 200 decode.

#[path = "h2_support/mod.rs"]
mod support;

use std::time::Duration;

use bytes::BytesMut;
use leyline::h2::config::{H2Config, PseudoOrder, SettingId};
use leyline::h2::connection::{ClientConnection, PseudoHeaders};
use leyline::h2::frame::{DataFrame, FrameType, HeadersFrame};
use leyline::h2::hpack;
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
        settings_ack_timeout: Duration::from_secs(10),
        max_response_body_bytes: 100 * 1024 * 1024,
        max_header_block_bytes: 256 * 1024,
        settings_flood_threshold: 100,
        settings_flood_window: Duration::from_secs(10),
    }
}

/// Write one HEADERS frame through a shared encoder (continuous HPACK).
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

        // Drain until the request HEADERS. A no-body GET carries END_STREAM,
        // so the request stream is HalfClosedLocal — the production path.
        let stream_id = loop {
            let (h, _) = read_frame(&mut server_io).await;
            if h.frame_type == FrameType::Headers as u8 {
                assert_ne!(h.flags & END_STREAM, 0, "no-body GET carries END_STREAM");
                break h.stream_id;
            }
        };

        // One shared encoder → HPACK dynamic-table continuity across both blocks.
        let mut enc = hpack::Encoder::new();
        // 103 Early Hints with a preload Link hint — must be discarded entirely.
        write_headers_block(
            &mut server_io,
            &mut enc,
            stream_id,
            &[(":status", "103"), ("link", "</style.css>; rel=preload")],
            false,
        )
        .await;
        // Final 200, no END_STREAM (DATA terminates the stream).
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
        };
        let mut buf = BytesMut::new();
        d.encode(&mut buf);
        server_io.write_all(&buf).await.expect("data write");
        let _ = server_io.shutdown().await;
    });

    let (handle, _driver) = ClientConnection::start(client_io, config())
        .await
        .expect("handshake");

    let resp = tokio::time::timeout(
        Duration::from_secs(5),
        handle.send_request(
            PseudoHeaders {
                method: "GET".into(),
                scheme: "https".into(),
                authority: "example.com".into(),
                path: "/".into(),
                protocol: None,
            },
            vec![],
            None,
        ),
    )
    .await
    .expect("request timed out")
    .expect("a 103 before the 200 must not error the request");

    // The 103 is provisional: the FINAL status wins.
    assert_eq!(
        resp.status, 200,
        "1xx Early Hints must be skipped, not returned as the final status"
    );
    assert_eq!(resp.body, b"<html>ok</html>");
    // The 200's real headers are present...
    assert!(
        resp.headers
            .iter()
            .any(|(k, v)| k == "content-type" && v == "text/html"),
        "final headers must come from the 200, got {:?}",
        resp.headers
    );
    // ...and the 103's provisional preload hint must NOT poison them.
    assert!(
        !resp.headers.iter().any(|(k, _)| k == "link"),
        "103 Early Hints headers must be discarded, got {:?}",
        resp.headers
    );

    server.await.expect("mock server panicked");
}
