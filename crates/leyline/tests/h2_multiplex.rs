//! Concurrent multiplexing tests for the driver/handle model.
//!
//! Each test spins up a tiny mock H2 server inline over `tokio::io::duplex`
//! and exercises one property of the driver: independent stream progress,
//! flow-control parking, graceful shutdown, and reader-EOF fan-out.

#[path = "h2_support/mod.rs"]
mod support;

use std::time::Duration;

use bytes::BytesMut;
use support::*;
use tokio::io::AsyncReadExt;

use leyline::h2::config::{H2Config, PseudoOrder, SettingId};
use leyline::h2::connection::{ClientConnection, PseudoHeaders};
use leyline::h2::frame::{FrameType, GoAwayFrame};

// A minimal Chrome-ish H2Config. We deliberately leave
// INITIAL_CONNECTION_WINDOW_SIZE at the protocol default (65535) so the
// client never emits the optional follow-up WINDOW_UPDATE — keeps the
// mock server script simple.
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
    }
}

fn get_req(
    path: &str,
) -> (
    PseudoHeaders,
    Vec<(
        std::borrow::Cow<'static, str>,
        std::borrow::Cow<'static, str>,
    )>,
) {
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

// ---------------------------------------------------------------------------
// Test 1: Two concurrent requests complete independently, out of order.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn two_concurrent_requests_respond_out_of_order() {
    let (client_io, mut server_io) = tokio::io::duplex(65_536);

    // Mock server.
    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        // client SETTINGS
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        // server SETTINGS
        write_server_settings(&mut server_io).await;
        // server SETTINGS ACK for the client's SETTINGS (RFC 9113 §6.5.3;
        // the client now enforces this with a timeout).
        write_settings_ack(&mut server_io).await;
        // client SETTINGS ACK
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        assert!(h.flags & 0x1 != 0, "expected SETTINGS ACK");

        // Two client HEADERS frames (stream 1, stream 3), both END_STREAM.
        let (h1, _) = read_frame(&mut server_io).await;
        assert_eq!(h1.frame_type, FrameType::Headers as u8);
        assert_eq!(h1.stream_id, 1);
        let (h2, _) = read_frame(&mut server_io).await;
        assert_eq!(h2.frame_type, FrameType::Headers as u8);
        assert_eq!(h2.stream_id, 3);

        // Respond to stream 3 first, then stream 1.
        write_response(&mut server_io, 3, b"second-response").await;
        write_response(&mut server_io, 1, b"first-response").await;

        // Drain further traffic (eg GOAWAY from graceful shutdown).
        let mut sink = [0u8; 256];
        let _ = server_io.read(&mut sink).await;
    });

    let (handle, _driver) = ClientConnection::start(client_io, test_config())
        .await
        .expect("handshake");

    let (p1, h1) = get_req("/one");
    let (p2, h2) = get_req("/two");

    let h1_clone = handle.clone();
    let h2_clone = handle.clone();
    let fut_a = tokio::spawn(async move { h1_clone.send_request(p1, h1, None).await });
    let fut_b = tokio::spawn(async move { h2_clone.send_request(p2, h2, None).await });

    let (a, b) = tokio::join!(fut_a, fut_b);
    let a = a.unwrap().expect("req a");
    let b = b.unwrap().expect("req b");
    assert_eq!(a.status, 200);
    assert_eq!(b.status, 200);
    assert_eq!(a.body, b"first-response");
    assert_eq!(b.body, b"second-response");

    drop(handle);
    let _ = server.await;
}

// ---------------------------------------------------------------------------
// Test 2: Stream A flow-control exhausted, stream B makes progress.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn parked_stream_does_not_block_others() {
    let (client_io, mut server_io) = tokio::io::duplex(65_536);

    // Mock server.
    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;

        // Client SETTINGS.
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);

        // Server advertises INITIAL_WINDOW_SIZE = 10.
        write_server_settings_with(&mut server_io, vec![(0x4, 10)]).await;

        // Client SETTINGS ACK.
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        assert!(h.flags & 0x1 != 0);

        // Our SETTINGS ACK.
        write_settings_ack(&mut server_io).await;

        // Request A: HEADERS + one DATA (capped at 10 bytes by the stream
        // window — the client should park after).
        let (ha, _) = read_frame(&mut server_io).await;
        assert_eq!(ha.frame_type, FrameType::Headers as u8);
        assert_eq!(ha.stream_id, 1);
        assert!(ha.flags & 0x1 == 0, "A HEADERS must not END_STREAM");

        // Request B HEADERS arrives (stream 3) and end-streams because the
        // body fits entirely under the 10-byte window.
        //
        // The driver may interleave A's first DATA before B's HEADERS or
        // vice versa — accept either order.
        let mut got_a_data_first = false;
        let (maybe_data_or_headers, p) = read_frame(&mut server_io).await;
        if maybe_data_or_headers.frame_type == FrameType::Data as u8
            && maybe_data_or_headers.stream_id == 1
        {
            got_a_data_first = true;
            // A's first 10-byte DATA chunk (not END_STREAM — 10 more to send).
            assert_eq!(p.len(), 10);
            assert!(maybe_data_or_headers.flags & 0x1 == 0);
            // Now B's HEADERS.
            let (hb, _) = read_frame(&mut server_io).await;
            assert_eq!(hb.frame_type, FrameType::Headers as u8);
            assert_eq!(hb.stream_id, 3);
        } else {
            assert_eq!(maybe_data_or_headers.frame_type, FrameType::Headers as u8);
            assert_eq!(maybe_data_or_headers.stream_id, 3);
        }

        // Now read whichever hasn't arrived yet: the A-DATA (if B-HEADERS
        // came first) or B-DATA (if A-DATA already consumed).
        let (nf, np) = read_frame(&mut server_io).await;
        if got_a_data_first {
            // Expect B-DATA now.
            assert_eq!(nf.frame_type, FrameType::Data as u8);
            assert_eq!(nf.stream_id, 3);
            assert_eq!(np.len(), 5);
            assert!(nf.flags & 0x1 != 0, "B DATA must END_STREAM");
        } else {
            // Expect A-DATA (10 bytes) first.
            assert_eq!(nf.frame_type, FrameType::Data as u8);
            assert_eq!(nf.stream_id, 1);
            assert_eq!(np.len(), 10);
            assert!(nf.flags & 0x1 == 0);
            // Then B-DATA (5 bytes, END_STREAM).
            let (bf, bp) = read_frame(&mut server_io).await;
            assert_eq!(bf.frame_type, FrameType::Data as u8);
            assert_eq!(bf.stream_id, 3);
            assert_eq!(bp.len(), 5);
            assert!(bf.flags & 0x1 != 0);
        }

        // Respond to B (short, fits in window).
        write_response(&mut server_io, 3, b"bbb").await;

        // Now unblock A: give it another 10 bytes of stream window + 20 bytes
        // of connection window (we consumed some).
        write_window_update(&mut server_io, 0, 64).await;
        write_window_update(&mut server_io, 1, 20).await;

        // Read A's remaining 10-byte DATA frame (END_STREAM).
        let (af, ap) = read_frame(&mut server_io).await;
        assert_eq!(af.frame_type, FrameType::Data as u8);
        assert_eq!(af.stream_id, 1);
        assert_eq!(ap.len(), 10);
        assert!(af.flags & 0x1 != 0, "A final DATA must END_STREAM");

        // Respond to A.
        write_response(&mut server_io, 1, b"aaa").await;

        // Drain shutdown.
        let mut sink = [0u8; 256];
        let _ = server_io.read(&mut sink).await;
    });

    let (handle, _driver) = ClientConnection::start(client_io, test_config())
        .await
        .expect("handshake");

    let (p_a, h_a) = get_req("/a");
    let (p_b, h_b) = get_req("/b");

    let body_a = bytes::Bytes::from_static(&[b'A'; 20]);
    let body_b = bytes::Bytes::from_static(&[b'B'; 5]);

    let ha = handle.clone();
    let hb = handle.clone();
    let fut_a = tokio::spawn(async move {
        ha.send_request(
            PseudoHeaders {
                method: "POST".into(),
                ..p_a
            },
            h_a,
            Some(body_a),
        )
        .await
    });
    let fut_b = tokio::spawn(async move {
        hb.send_request(
            PseudoHeaders {
                method: "POST".into(),
                ..p_b
            },
            h_b,
            Some(body_b),
        )
        .await
    });

    let b_result = tokio::time::timeout(Duration::from_secs(3), fut_b)
        .await
        .expect("B timeout");
    let b = b_result.unwrap().expect("req B");
    assert_eq!(b.status, 200);
    assert_eq!(b.body, b"bbb");

    let a_result = tokio::time::timeout(Duration::from_secs(3), fut_a)
        .await
        .expect("A timeout");
    let a = a_result.unwrap().expect("req A");
    assert_eq!(a.status, 200);
    assert_eq!(a.body, b"aaa");

    drop(handle);
    let _ = server.await;
}

// ---------------------------------------------------------------------------
// Test 3: Driver shuts down cleanly when the last handle drops.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn driver_shuts_down_on_last_handle_drop() {
    let (client_io, mut server_io) = tokio::io::duplex(4096);

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        let (_h, _) = read_frame(&mut server_io).await; // client SETTINGS
        write_server_settings(&mut server_io).await;
        write_settings_ack(&mut server_io).await; // ACK the client's SETTINGS
        let (_h, _) = read_frame(&mut server_io).await; // client SETTINGS ACK

        // Wait for GOAWAY.
        let (hdr, payload) = read_frame(&mut server_io).await;
        assert_eq!(hdr.frame_type, FrameType::GoAway as u8);
        let g = GoAwayFrame::parse(hdr, bytes::Bytes::from(payload)).unwrap();
        assert_eq!(g.error_code as u32, 0, "NO_ERROR goaway expected");
    });

    let (handle, driver) = ClientConnection::start(client_io, test_config())
        .await
        .expect("handshake");

    let clone1 = handle.clone();
    let clone2 = handle.clone();
    drop(handle);
    drop(clone1);
    drop(clone2);

    // Driver should complete.
    tokio::time::timeout(Duration::from_secs(2), driver.join())
        .await
        .expect("driver join timeout")
        .expect("driver error");

    let _ = tokio::time::timeout(Duration::from_secs(1), server).await;
}

// ---------------------------------------------------------------------------
// Test 4: Reader EOF fails every pending request.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reader_eof_fails_pending_requests() {
    let (client_io, mut server_io) = tokio::io::duplex(4096);

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        let (_h, _) = read_frame(&mut server_io).await; // client SETTINGS
        write_server_settings(&mut server_io).await;
        write_settings_ack(&mut server_io).await; // ACK the client's SETTINGS
        let (_h, _) = read_frame(&mut server_io).await; // client SETTINGS ACK

        // Read the two HEADERS frames and drop the connection.
        let (_a, _) = read_frame(&mut server_io).await;
        let (_b, _) = read_frame(&mut server_io).await;

        // Drop the server side — client sees EOF.
        drop(server_io);
    });

    let (handle, _driver) = ClientConnection::start(client_io, test_config())
        .await
        .expect("handshake");

    let (p1, h1) = get_req("/one");
    let (p2, h2) = get_req("/two");

    let ha = handle.clone();
    let hb = handle.clone();
    let fut_a = tokio::spawn(async move { ha.send_request(p1, h1, None).await });
    let fut_b = tokio::spawn(async move { hb.send_request(p2, h2, None).await });

    let (a, b) = tokio::time::timeout(Duration::from_secs(3), async { tokio::join!(fut_a, fut_b) })
        .await
        .expect("joins timeout");

    assert!(a.unwrap().is_err(), "req A must fail on EOF");
    assert!(b.unwrap().is_err(), "req B must fail on EOF");

    let _ = tokio::time::timeout(Duration::from_secs(1), server).await;
    // Silence unused warning from dev deps.
    let _ = BytesMut::new();
}
