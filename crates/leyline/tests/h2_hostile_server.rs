//! Adversarial HTTP/2 server tests: a hostile or buggy peer must not be able
//! to make the client mis-behave. These drive the real `ClientConnection`
//! driver against hand-crafted server frames (the driver's own frame handling
//! is under test, so the server side emits raw RFC 9113 bytes).
//!
//! Every test drives the client against attacker-controlled bytes and
//! reproduces a specific way a hostile server could mis-behave it.
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

/// Regression: the inbound frame-size cap must be *our* advertised
/// SETTINGS_MAX_FRAME_SIZE (16384), never the peer's. Previously the driver
/// set the reader's cap from `peer_settings.max_frame_size` — so a hostile
/// server could advertise a 16 MB frame size and make the client accept (and
/// eagerly `BytesMut::zeroed`-preallocate) frames far larger than it ever
/// agreed to receive. The client must instead reject the oversized frame with
/// `FrameTooLarge` at *its own* limit.
#[tokio::test]
async fn peer_max_frame_size_does_not_raise_our_inbound_cap() {
    let (client_io, mut server_io) = tokio::io::duplex(1 << 20);

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        // Advertise a huge MAX_FRAME_SIZE (2^24 - 1, the RFC max). Under the
        // old bug the client would adopt this as its own inbound cap.
        write_server_settings_with(&mut server_io, vec![(0x5, 0x00FF_FFFF)]).await;
        write_settings_ack(&mut server_io).await;
        // Client's SETTINGS ack.
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        assert!(h.flags & 0x1 != 0, "expected client SETTINGS ack");
        // Client's request HEADERS on stream 1.
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Headers as u8);
        assert_eq!(h.stream_id, 1);
        // Send a DATA frame header declaring length 65536 — over our advertised
        // 16384, well under the peer's advertised max. A raw 9-byte frame
        // header is enough: the reader rejects on the length field before it
        // reads (or allocates) any payload.
        //   length = 0x010000, type = 0x00 (DATA), flags = 0, stream_id = 1
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

/// Regression: a response HEADERS block with a missing or malformed `:status`
/// must fail only *that* stream (RFC 9113 §8.3.1 malformed response) — the
/// multiplexed connection must survive, and a later request on it must still
/// succeed. Previously a missing `:status` was delivered to the caller as a
/// synthetic `status = 0` "success", and a malformed one propagated `?` out of
/// the event loop and tore down every stream on the connection.
#[tokio::test]
async fn bad_status_fails_stream_but_not_connection() {
    for bad_headers in [
        vec![("content-type", "text/plain")], // no :status at all
        vec![(":status", "two-hundred")],     // non-numeric :status
        vec![(":status", "20")],              // not three digits
        vec![(":status", "2000")],            // not three digits
        vec![(":status", "101")],             // forbidden in HTTP/2
        vec![(":status", "200"), (":status", "204")], // duplicate :status
        vec![("content-type", "text/plain"), (":status", "200")], // pseudo after regular
        vec![(":status", "200"), (":method", "GET")], // request pseudo-header
        vec![(":status", "200"), (":unknown", "value")], // unknown pseudo-header
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

            // Request A on stream 1 → reply with the bad header block.
            let (h, _) = read_frame(&mut server_io).await;
            assert_eq!(h.frame_type, FrameType::Headers as u8);
            assert_eq!(h.stream_id, 1);
            write_raw_headers(&mut server_io, 1, &bad_headers, true).await;

            // Request B on stream 3 → a well-formed 200, proving the connection
            // stayed alive after A's stream-level failure. The client RSTs the
            // just-failed stream 1, so skip any non-HEADERS frames (RST_STREAM /
            // WINDOW_UPDATE) until B's HEADERS arrive.
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

        // Request A must fail as a stream error, not a status-0 "success".
        let (p, h) = get_req("/a");
        let a = handle.send_request(p, h, None).await;
        assert!(a.is_err(), "a bad :status must fail the stream, got {a:?}");

        // Request B must still succeed — the connection survived A's failure.
        let (p, h) = get_req("/b");
        let b = handle
            .send_request(p, h, None)
            .await
            .expect("connection must survive a malformed-response stream error");
        assert_eq!(b.status, 200);

        server.abort();
    }
}

/// Regression: a trailer HEADERS block (a second header block after the
/// response headers) that arrives WITHOUT END_STREAM is malformed per
/// RFC 9113 §8.1 — trailers are the last thing on the stream. It previously
/// completed the stream as if the response had ended cleanly; it must now fail
/// the stream instead.
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

        // Response headers (200), deliberately WITHOUT END_STREAM so the stream
        // stays open and the next HEADERS block is treated as trailers.
        write_raw_headers(&mut server_io, 1, &[(":status", "200")], false).await;
        // Trailer block, also without END_STREAM — malformed.
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

/// A trailer section must not contain pseudo-headers, even when it correctly
/// terminates the stream. RFC 9113 §8.3.1 reserves pseudo-headers for the
/// initial header block.
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

/// A PUSH_PROMISE field block mutates the shared HPACK table even when
/// the promised stream is reset — it must be decoded regardless (RFC 9113
/// §4.3), or the next dynamic-index reference kills the connection.
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

        // Request A on stream 1. Push promised stream 2 with a field block
        // that inserts (x, y) via literal-with-incremental-indexing, then
        // answer stream 1 referencing that fresh dynamic entry (62) with
        // END_STREAM.
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Headers as u8);
        assert_eq!(h.stream_id, 1);

        server_io
            .write_all(&[
                0x00, 0x00, 0x09, 0x05, 0x04, 0x00, 0x00, 0x00, 0x02, // hdr
                0x00, 0x00, 0x00, 0x02, // promised stream 2
                0x40, 0x01, b'x', 0x01, b'y', // insert "x: y"
            ])
            .await
            .unwrap();
        server_io
            .write_all(&[
                0x00, 0x00, 0x02, 0x01, 0x05, 0x00, 0x00, 0x00, 0x01, // hdr
                0x88, 0xbe, // :status 200 + indexed 62 = (x, y)
            ])
            .await
            .unwrap();

        // The client must still RST the promised stream.
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
