//! Regression gate for the stream-level receive window.
//!
//! The driver used to seed each stream's recv window from
//! `initial_connection_window_size` (our connection-level value) and
//! top it up against `peer_settings.initial_window_size` (the PEER's
//! stream value, which governs the send direction). Neither is the
//! value we advertise to the server in our own SETTINGS frame, so a
//! server that honours flow control stalls after exhausting our
//! advertised stream window on any body larger than it — the client
//! never sends the stream-level WINDOW_UPDATE the server is waiting
//! for.
//!
//! The mock server here plays strictly by RFC 9113 §5.2: it tracks the
//! client's advertised stream window plus the connection window and
//! refuses to send DATA past either credit until a WINDOW_UPDATE
//! arrives. A body larger than the advertised stream window therefore
//! completes only if the client replenishes the stream window.

#[path = "h2_support/mod.rs"]
mod support;

use std::time::Duration;

use bytes::BytesMut;
use leyline::h2::config::{H2Config, PseudoOrder, SettingId};
use leyline::h2::connection::{ClientConnection, PseudoHeaders};
use leyline::h2::frame::{DataFrame, FrameType, HeadersFrame};
use leyline::h2::hpack;
use support::*;
use tokio::io::AsyncWriteExt;

/// Our advertised per-stream receive window.
const STREAM_WINDOW: u32 = 65_535;
/// Connection-level window — deliberately much larger than the stream
/// window so only the stream credit gates the transfer (and so the
/// buggy connection-seeded recv window visibly diverges).
const CONN_WINDOW: u32 = 1_048_576;
/// Response body size — far past the stream window so the server must
/// pause for a stream WINDOW_UPDATE at least once.
const BODY_LEN: usize = 200_000;
const MAX_FRAME: usize = 16_384;

fn window_config() -> H2Config {
    H2Config {
        settings: vec![
            (SettingId::HeaderTableSize, 4096),
            (SettingId::EnablePush, 0),
            (SettingId::InitialWindowSize, STREAM_WINDOW),
            (SettingId::MaxFrameSize, MAX_FRAME as u32),
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
        initial_connection_window_size: CONN_WINDOW,
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

async fn write_response_headers<S: tokio::io::AsyncWrite + Unpin>(s: &mut S, stream_id: u32) {
    let mut enc = hpack::Encoder::new();
    let fragment = enc.encode_header_block(&[(":status", "200")]);
    let h = HeadersFrame {
        stream_id,
        end_stream: false,
        end_headers: true,
        priority: None,
        fragment: bytes::Bytes::from(fragment),
    };
    let mut buf = BytesMut::new();
    h.encode(&mut buf);
    s.write_all(&buf).await.expect("resp headers write");
}

#[tokio::test]
async fn large_body_completes_against_flow_control_honouring_server() {
    let (client_io, mut server_io) = tokio::io::duplex(1 << 20);

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8, "client SETTINGS");
        // Advertise RFC defaults explicitly; the peer's stream window
        // governs the client's SEND direction only.
        write_server_settings_with(&mut server_io, vec![(0x4, 65_535)]).await;
        write_settings_ack(&mut server_io).await;

        // Credits the client granted us: its advertised stream window
        // plus the connection window (65 535 base + any conn-level
        // WINDOW_UPDATE it sends after the preface).
        let mut stream_credit: u64 = STREAM_WINDOW as u64;
        let mut conn_credit: u64 = 65_535;

        // Drain frames until the request HEADERS arrives, crediting
        // any connection-level WINDOW_UPDATE seen on the way.
        let stream_id = loop {
            let (h, payload) = read_frame(&mut server_io).await;
            match h.frame_type {
                t if t == FrameType::WindowUpdate as u8 => {
                    let inc = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]])
                        & 0x7FFF_FFFF;
                    if h.stream_id == 0 {
                        conn_credit += inc as u64;
                    } else {
                        stream_credit += inc as u64;
                    }
                }
                t if t == FrameType::Headers as u8 => break h.stream_id,
                _ => {} // SETTINGS ack, PING, PRIORITY — ignore.
            }
        };

        write_response_headers(&mut server_io, stream_id).await;

        let mut sent: usize = 0;
        while sent < BODY_LEN {
            let budget = (BODY_LEN - sent)
                .min(MAX_FRAME)
                .min(stream_credit as usize)
                .min(conn_credit as usize);
            if budget == 0 {
                // Out of credit. A correct client replenishes the
                // stream window; the buggy one never does.
                let (h, payload) =
                    tokio::time::timeout(Duration::from_secs(3), read_frame(&mut server_io))
                        .await
                        .expect(
                            "server starved: client never sent the stream WINDOW_UPDATE \
                     needed to finish the body",
                        );
                if h.frame_type == FrameType::WindowUpdate as u8 {
                    let inc = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]])
                        & 0x7FFF_FFFF;
                    if h.stream_id == 0 {
                        conn_credit += inc as u64;
                    } else {
                        stream_credit += inc as u64;
                    }
                }
                continue;
            }

            let end_stream = sent + budget == BODY_LEN;
            let d = DataFrame {
                stream_id,
                end_stream,
                data: bytes::Bytes::from(vec![0xAB; budget]),
            };
            let mut buf = BytesMut::new();
            d.encode(&mut buf);
            server_io.write_all(&buf).await.expect("data write");
            sent += budget;
            stream_credit -= budget as u64;
            conn_credit -= budget as u64;

            // Opportunistically drain any WINDOW_UPDATE / ack frames the
            // client pushed while we were writing, without blocking.
            while let Ok((h, payload)) =
                tokio::time::timeout(Duration::from_millis(5), read_frame(&mut server_io)).await
            {
                if h.frame_type == FrameType::WindowUpdate as u8 {
                    let inc = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]])
                        & 0x7FFF_FFFF;
                    if h.stream_id == 0 {
                        conn_credit += inc as u64;
                    } else {
                        stream_credit += inc as u64;
                    }
                }
            }
        }
    });

    let (handle, _driver) = ClientConnection::start(client_io, window_config())
        .await
        .expect("handshake");

    let resp = tokio::time::timeout(
        Duration::from_secs(10),
        handle.send_request(
            PseudoHeaders {
                method: "GET".into(),
                scheme: "https".into(),
                authority: "example.com".into(),
                path: "/big".into(),
                protocol: None,
            },
            vec![("user-agent".into(), "test".into())],
            None,
        ),
    )
    .await
    .expect("request timed out: body stalled at the advertised stream window")
    .expect("request failed");

    assert_eq!(resp.status, 200);
    assert_eq!(
        resp.body.len(),
        BODY_LEN,
        "body truncated — stream window was not replenished"
    );

    server.await.expect("mock server panicked");
}
