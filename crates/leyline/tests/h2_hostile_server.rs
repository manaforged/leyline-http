//! Adversarial HTTP/2 server tests: a hostile or buggy peer must not be able to make the client mis-behave.
#[path = "h2_support/mod.rs"]
mod support;

use std::borrow::Cow;
use std::time::Duration;

use leyline::h2::config::{H2Config, PseudoOrder, SettingId};
use leyline::h2::connection::{ClientConnection, PseudoHeaders};
use leyline::h2::frame::FrameType;
use support::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
        settings_flood_window: Duration::from_secs(10),
        header_block_reassembly_timeout: Duration::from_secs(10),
    }
}

type CowHeaders = Vec<(Cow<'static, str>, Cow<'static, str>)>;

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

/// Regression: the inbound frame-size cap must be *our* advertised SETTINGS_MAX_FRAME_SIZE (16384), never the peer's.
#[tokio::test]
async fn peer_max_frame_size_does_not_raise_our_inbound_cap() {
    let (client_io, mut server_io) = tokio::io::duplex(1 << 20);

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        write_server_settings_with(&mut server_io, vec![(0x5, 0x00FF_FFFF)]).await;
        write_settings_ack(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        assert!(h.flags & 0x1 != 0, "expected client SETTINGS ack");
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Headers as u8);
        assert_eq!(h.stream_id, 1);
        let oversized = [0x01u8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01];
        let _ = server_io.write_all(&oversized).await;
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
        .expect("driver must terminate on an oversized frame")
        .expect_err("driver must report an error");
    match err {
        leyline::h2::H2Error::FrameTooLarge { size, max } => {
            assert_eq!(
                max, 16384,
                "inbound cap must be OUR advertised MAX_FRAME_SIZE, not the peer's"
            );
            assert_eq!(size, 65536, "should report the offending declared size");
        }
        other => panic!("expected FrameTooLarge, got {other:?}"),
    }

    let _ = req_fut.await;
    server.abort();
}

/// Regression: a response HEADERS block with a missing or malformed `:status` must fail only *that* stream (RFC 9113 §8.3.1 malformed response) — the multiplexed connection must survive, and a later request on it must still succeed.
#[tokio::test]
async fn bad_status_fails_stream_but_not_connection() {
    for bad_headers in [
        vec![("content-type", "text/plain")],
        vec![(":status", "two-hundred")],
        vec![(":status", "20")],
        vec![(":status", "2000")],
        vec![(":status", "101")],
        vec![(":status", "200"), (":status", "204")],
        vec![("content-type", "text/plain"), (":status", "200")],
        vec![(":status", "200"), (":method", "GET")],
        vec![(":status", "200"), (":unknown", "value")],
    ] {
        let (client_io, mut server_io) = tokio::io::duplex(65_536);

        let server = tokio::spawn(async move {
            read_preface(&mut server_io).await;
            let (h, _) = read_frame(&mut server_io).await;
            assert_eq!(h.frame_type, FrameType::Settings as u8);
            write_server_settings(&mut server_io).await;
            write_settings_ack(&mut server_io).await;
            let (h, _) = read_frame(&mut server_io).await;
            assert_eq!(h.frame_type, FrameType::Settings as u8);
            assert!(h.flags & 0x1 != 0, "expected client SETTINGS ack");

            let (h, _) = read_frame(&mut server_io).await;
            assert_eq!(h.frame_type, FrameType::Headers as u8);
            assert_eq!(h.stream_id, 1);
            write_raw_headers(&mut server_io, 1, &bad_headers, true).await;

            let hb = loop {
                let (fh, _) = read_frame(&mut server_io).await;
                if fh.frame_type == FrameType::Headers as u8 {
                    break fh;
                }
            };
            assert_eq!(hb.stream_id, 3, "expected request B on stream 3");
            write_response(&mut server_io, 3, b"ok").await;

            let mut sink = [0u8; 256];
            let _ = server_io.read(&mut sink).await;
        });

        let (handle, _driver) = ClientConnection::start(client_io, test_config())
            .await
            .expect("handshake");

        let (p, h) = get_req("/a");
        let a = handle.send_request(p, h, None).await;
        assert!(a.is_err(), "a bad :status must fail the stream, got {a:?}");

        let (p, h) = get_req("/b");
        let b = handle
            .send_request(p, h, None)
            .await
            .expect("connection must survive a malformed-response stream error");
        assert_eq!(b.status, 200);

        server.abort();
    }
}

/// Regression: a trailer HEADERS block (a second header block after the response headers) that arrives WITHOUT END_STREAM is malformed per RFC 9113 §8.1 — trailers are the last thing on the stream.
#[tokio::test]
async fn trailers_without_end_stream_fail_the_stream() {
    let (client_io, mut server_io) = tokio::io::duplex(65_536);

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        write_server_settings(&mut server_io).await;
        write_settings_ack(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        assert!(h.flags & 0x1 != 0, "expected client SETTINGS ack");

        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Headers as u8);
        assert_eq!(h.stream_id, 1);

        write_raw_headers(&mut server_io, 1, &[(":status", "200")], false).await;
        write_raw_headers(&mut server_io, 1, &[("x-trailer", "late")], false).await;

        let mut sink = [0u8; 256];
        let _ = server_io.read(&mut sink).await;
    });

    let (handle, _driver) = ClientConnection::start(client_io, test_config())
        .await
        .expect("handshake");

    let (p, h) = get_req("/");
    let result = handle.send_request(p, h, None).await;
    assert!(
        result.is_err(),
        "trailers without END_STREAM must fail the stream, got {result:?}"
    );

    server.abort();
}

/// A trailer section must not contain pseudo-headers, even when it correctly terminates the stream.
#[tokio::test]
async fn pseudo_header_in_trailers_fails_the_stream() {
    let (client_io, mut server_io) = tokio::io::duplex(65_536);

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        write_server_settings(&mut server_io).await;
        write_settings_ack(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        assert!(h.flags & 0x1 != 0, "expected client SETTINGS ack");

        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Headers as u8);
        assert_eq!(h.stream_id, 1);
        write_raw_headers(&mut server_io, 1, &[(":status", "200")], false).await;
        write_raw_headers(&mut server_io, 1, &[(":status", "204")], true).await;

        let mut sink = [0_u8; 256];
        let _ = server_io.read(&mut sink).await;
    });

    let (handle, _driver) = ClientConnection::start(client_io, test_config())
        .await
        .expect("handshake");

    let (p, h) = get_req("/");
    let result = handle.send_request(p, h, None).await;
    assert!(
        result.is_err(),
        "trailers with pseudo-headers must fail the stream, got {result:?}"
    );

    server.abort();
}

/// A PUSH_PROMISE field block mutates the shared HPACK table even when the promised stream is reset — it must be decoded regardless (RFC 9113 §4.3), or the next dynamic-index reference kills the connection.
#[tokio::test]
async fn push_promise_field_block_is_hpack_decoded_before_reset() {
    let (client_io, mut server_io) = tokio::io::duplex(65_536);

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        write_server_settings(&mut server_io).await;
        write_settings_ack(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        assert!(h.flags & 0x1 != 0, "expected client SETTINGS ack");

        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Headers as u8);
        assert_eq!(h.stream_id, 1);

        server_io
            .write_all(&[
                0x00, 0x00, 0x09, 0x05, 0x04, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x40,
                0x01, b'x', 0x01, b'y',
            ])
            .await
            .unwrap();
        server_io
            .write_all(&[
                0x00, 0x00, 0x02, 0x01, 0x05, 0x00, 0x00, 0x00, 0x01, 0x88, 0xbe,
            ])
            .await
            .unwrap();

        loop {
            let (fh, _) = read_frame(&mut server_io).await;
            if fh.frame_type == FrameType::RstStream as u8 && fh.stream_id == 2 {
                break;
            }
        }
    });

    let (handle, _driver) = ClientConnection::start(client_io, test_config())
        .await
        .expect("handshake");

    let (p, h) = get_req("/a");
    let resp = handle
        .send_request(p, h, None)
        .await
        .expect("push + dynamic reference must decode cleanly");

    assert_eq!(resp.status, 200);
    let x = resp
        .headers
        .iter()
        .find(|(n, _)| n.as_str() == "x")
        .map(|(_, v)| v.as_str());
    assert_eq!(x, Some("y"), "pushed-entry reference must decode");

    server.await.unwrap();
}
