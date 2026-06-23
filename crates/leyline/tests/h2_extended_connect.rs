//! Integration tests for RFC 8441 extended CONNECT (WebSocket-over-H2).
//!
//! Each test stands up a bespoke mock H2 server inline over
//! `tokio::io::duplex`. We exercise:
//!
//! - the happy path: peer advertises `ENABLE_CONNECT_PROTOCOL=1`,
//!   server echoes DATA frames, client observes them;
//! - the fallback trigger: peer omits the setting, client's
//!   [`H2Client::open_extended_connect`] errors with a shape that
//!   [`leyline::core::websocket::WsConnection::is_h2_fallback_trigger`]
//!   treats as "try H1";
//! - stream teardown: dropping the stream closes the write half with
//!   END_STREAM and leaves sibling streams alive on the connection.

#[path = "h2_support/mod.rs"]
mod support;

use std::time::Duration;

use bytes::{Bytes, BytesMut};
use support::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use leyline::h2::codec::FrameReader;
use leyline::h2::config::{H2Config, PseudoOrder, SettingId};
use leyline::h2::connection::{ClientConnection, PseudoHeaders};
use leyline::h2::frame::{DataFrame, Frame, FrameType, HeadersFrame};
use leyline::h2::hpack;

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

fn connect_pseudo() -> (
    PseudoHeaders,
    Vec<(
        std::borrow::Cow<'static, str>,
        std::borrow::Cow<'static, str>,
    )>,
) {
    (
        PseudoHeaders {
            method: "CONNECT".into(),
            scheme: "https".into(),
            authority: "example.com:443".into(),
            path: "/chat".into(),
            protocol: Some("websocket".into()),
        },
        vec![
            ("sec-websocket-version".into(), "13".into()),
            (
                "sec-websocket-key".into(),
                "dGhlIHNhbXBsZSBub25jZQ==".into(),
            ),
            ("user-agent".into(), "leyline-test".into()),
        ],
    )
}

async fn perform_handshake<S>(server_io: &mut S, advertise_connect: bool)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    read_preface(server_io).await;
    let (h, _) = read_frame(server_io).await;
    assert_eq!(h.frame_type, FrameType::Settings as u8);

    let params = if advertise_connect {
        // RFC 8441: SETTINGS_ENABLE_CONNECT_PROTOCOL = 0x8, value 1.
        vec![(0x8u16, 1u32)]
    } else {
        vec![]
    };
    write_server_settings_with(server_io, params).await;
    write_settings_ack(server_io).await;

    let (h, _) = read_frame(server_io).await;
    assert_eq!(h.frame_type, FrameType::Settings as u8);
    assert!(h.flags & 0x1 != 0, "expected client SETTINGS ACK");
}

/// Decode a HEADERS / CONTINUATION sequence from the stream, returning
/// the assembled header list for assertions.
async fn read_header_block<S>(server_io: &mut S) -> (u32, bool, Vec<(String, String)>)
where
    S: tokio::io::AsyncRead + Unpin,
{
    let (h, payload) = read_frame(server_io).await;
    assert_eq!(h.frame_type, FrameType::Headers as u8);
    let stream_id = h.stream_id;
    let end_stream = h.flags & 0x1 != 0;
    let end_headers = h.flags & 0x4 != 0;
    let priority = h.flags & 0x20 != 0;
    // Skip optional PAD_LENGTH (we don't emit padding in the client).
    let start = if priority { 5 } else { 0 };
    let mut fragment = payload[start..].to_vec();
    if !end_headers {
        loop {
            let (h2, p2) = read_frame(server_io).await;
            assert_eq!(h2.frame_type, FrameType::Continuation as u8);
            fragment.extend_from_slice(&p2);
            if h2.flags & 0x4 != 0 {
                break;
            }
        }
    }
    let mut dec = hpack::Decoder::new();
    let decoded = dec.decode_header_block(&fragment).expect("hpack decode");
    let list = decoded
        .into_iter()
        .map(|h| {
            (
                String::from_utf8_lossy(&h.name).into_owned(),
                String::from_utf8_lossy(&h.value).into_owned(),
            )
        })
        .collect::<Vec<_>>();
    (stream_id, end_stream, list)
}

/// Encode and write a 200 OK response HEADERS frame (non-end-stream)
/// for an extended CONNECT stream.
async fn write_connect_200<S: tokio::io::AsyncWrite + Unpin>(s: &mut S, stream_id: u32) {
    let mut enc = hpack::Encoder::new();
    let fragment = enc.encode_header_block(&[(":status", "200")]);
    let h = HeadersFrame {
        stream_id,
        end_stream: false,
        end_headers: true,
        priority: None,
        fragment: Bytes::from(fragment),
    };
    let mut buf = BytesMut::new();
    h.encode(&mut buf);
    s.write_all(&buf).await.expect("connect 200 write");
}

async fn write_data<S: tokio::io::AsyncWrite + Unpin>(
    s: &mut S,
    stream_id: u32,
    data: &[u8],
    end_stream: bool,
) {
    let d = DataFrame {
        stream_id,
        end_stream,
        data: Bytes::copy_from_slice(data),
    };
    let mut buf = BytesMut::new();
    d.encode(&mut buf);
    s.write_all(&buf).await.expect("data write");
}

// ---------------------------------------------------------------------------
// Test 1: happy path — server advertises setting, echoes a payload.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn h2_extended_connect_happy_path_echoes_payload() {
    let (client_io, mut server_io) = tokio::io::duplex(65_536);

    let server = tokio::spawn(async move {
        perform_handshake(&mut server_io, true).await;

        // Expect an extended-CONNECT HEADERS frame (no END_STREAM).
        let (sid, end_stream, hdrs) = read_header_block(&mut server_io).await;
        assert_eq!(sid, 1);
        assert!(!end_stream, "extended CONNECT must not send END_STREAM");
        assert_eq!(
            hdrs.iter()
                .find(|(n, _)| n == ":method")
                .map(|(_, v)| v.as_str()),
            Some("CONNECT")
        );
        assert_eq!(
            hdrs.iter()
                .find(|(n, _)| n == ":protocol")
                .map(|(_, v)| v.as_str()),
            Some("websocket")
        );
        assert!(
            hdrs.iter().any(|(n, _)| n == "sec-websocket-version"),
            "Sec-WebSocket-Version header missing: {hdrs:?}"
        );

        // Accept with :status 200.
        write_connect_200(&mut server_io, sid).await;

        // Read one DATA frame from the client, echo it back.
        let mut reader = FrameReader::new(tokio::io::BufReader::new(&mut server_io));
        let frame = reader
            .next()
            .await
            .expect("frame read")
            .expect("some frame");
        let data_bytes = match frame {
            Frame::Data(d) => {
                assert_eq!(d.stream_id, sid);
                assert!(!d.end_stream);
                d.data
            }
            other => panic!("expected DATA frame, got {other:?}"),
        };
        // End the reader's borrow so we can take the server_io write path.
        #[allow(dropping_references, clippy::drop_non_drop)]
        drop(reader);

        write_data(&mut server_io, sid, &data_bytes, false).await;

        // Give the client time to surface the chunk and drop the
        // stream; then drain whatever it sends.
        let mut sink = [0u8; 4096];
        let _ = tokio::time::timeout(Duration::from_millis(100), server_io.read(&mut sink)).await;
    });

    let (handle, _driver) = ClientConnection::start(client_io, test_config())
        .await
        .expect("handshake");

    // Wait for the server's SETTINGS (with ENABLE_CONNECT_PROTOCOL=1) to
    // have been applied by the driver. The handle's cached snapshot is
    // updated as soon as the driver processes the server's SETTINGS.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    while !handle.peer_enables_connect_protocol() {
        if tokio::time::Instant::now() >= deadline {
            panic!("peer never advertised ENABLE_CONNECT_PROTOCOL");
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let (pseudo, headers) = connect_pseudo();
    let mut stream = handle
        .open_extended_connect(pseudo, headers)
        .await
        .expect("extended CONNECT should succeed");
    assert_eq!(stream.status(), 200);

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    stream.write_all(b"hello h2 ws").await.expect("write");
    stream.flush().await.expect("flush");

    let mut buf = [0u8; 32];
    let n = stream.read(&mut buf).await.expect("read");
    assert_eq!(&buf[..n], b"hello h2 ws");

    drop(stream);
    server.await.expect("server task");
}

// ---------------------------------------------------------------------------
// Test 2: server omits ENABLE_CONNECT_PROTOCOL — client call errors,
// and the error is tagged for H1 fallback by core's WsConnection.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn h2_without_connect_protocol_falls_back() {
    let (client_io, mut server_io) = tokio::io::duplex(65_536);

    let server = tokio::spawn(async move {
        perform_handshake(&mut server_io, false).await;
        // Nothing further expected — client must not send a CONNECT
        // HEADERS frame on this connection. Read anything that arrives
        // and assert it isn't a CONNECT.
        let mut sink = [0u8; 4096];
        let res = tokio::time::timeout(Duration::from_millis(150), server_io.read(&mut sink)).await;
        if let Ok(Ok(n)) = res {
            if n > 0 {
                // The only legitimate traffic is a GOAWAY from graceful
                // shutdown when the handle drops. Decode & verify.
                // Frame type 0x7 == GOAWAY.
                assert_eq!(
                    sink[3], 0x7,
                    "expected GOAWAY (0x7), got frame type 0x{:02x}",
                    sink[3]
                );
            }
        }
    });

    let (handle, _driver) = ClientConnection::start(client_io, test_config())
        .await
        .expect("handshake");

    // Flush: ensure SETTINGS exchange has completed before we probe.
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        !handle.peer_enables_connect_protocol(),
        "peer must not have enabled connect protocol"
    );

    let (pseudo, headers) = connect_pseudo();
    let err = handle
        .open_extended_connect(pseudo, headers)
        .await
        .expect_err("extended CONNECT must fail without peer setting");

    // The H2Error message must mention the setting so upstream layers
    // (leyline-core::websocket) can detect and fall back.
    let msg = format!("{err}");
    assert!(
        msg.contains("SETTINGS_ENABLE_CONNECT_PROTOCOL") || msg.contains("ENABLE_CONNECT_PROTOCOL"),
        "error should name the missing setting, got: {msg}"
    );

    drop(handle);
    let _ = tokio::time::timeout(Duration::from_millis(200), server).await;
}

// ---------------------------------------------------------------------------
// Test 3: dropping the stream signals END_STREAM to the peer while a
// sibling stream on the same H2 connection continues to work.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dropping_connect_stream_signals_end_stream() {
    let (client_io, mut server_io) = tokio::io::duplex(65_536);

    let server = tokio::spawn(async move {
        perform_handshake(&mut server_io, true).await;

        // Extended CONNECT on stream 1.
        let (sid1, end, _hdrs) = read_header_block(&mut server_io).await;
        assert_eq!(sid1, 1);
        assert!(!end, "extended CONNECT must not carry END_STREAM");
        write_connect_200(&mut server_io, sid1).await;

        // The client-side driver emits an END_STREAM DATA frame when
        // the `H2ConnectStream` handle is dropped (via the relay task
        // seeing its mpsc sender closed). We read frames until we
        // observe that, with a budget for noise (WINDOW_UPDATE,
        // sibling streams).
        let mut reader = FrameReader::new(&mut server_io);
        let mut saw_end_stream = false;
        let mut saw_sibling = None::<u32>;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(1_000);
        while tokio::time::Instant::now() < deadline && (!saw_end_stream || saw_sibling.is_none()) {
            let frame = match tokio::time::timeout(Duration::from_millis(200), reader.next()).await
            {
                Ok(Ok(Some(f))) => f,
                Ok(Ok(None)) => break,
                _ => continue,
            };
            match frame {
                Frame::Data(d) if d.stream_id == sid1 && d.end_stream => {
                    saw_end_stream = true;
                }
                Frame::Headers(h) => {
                    saw_sibling = Some(h.stream_id);
                    #[allow(dropping_references, clippy::drop_non_drop)]
                    drop(reader);
                    write_response(&mut server_io, saw_sibling.unwrap(), b"sibling-ok").await;
                    reader = FrameReader::new(&mut server_io);
                }
                _ => {}
            }
        }
        (saw_end_stream, saw_sibling)
    });

    let (handle, _driver) = ClientConnection::start(client_io, test_config())
        .await
        .expect("handshake");

    // Wait for ENABLE_CONNECT_PROTOCOL=1.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    while !handle.peer_enables_connect_protocol() {
        if tokio::time::Instant::now() >= deadline {
            panic!("peer never advertised ENABLE_CONNECT_PROTOCOL");
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // Open the CONNECT stream, then immediately drop it to trigger
    // the graceful END_STREAM path.
    let (pseudo, headers) = connect_pseudo();
    let stream = handle
        .open_extended_connect(pseudo, headers)
        .await
        .expect("CONNECT open");
    drop(stream);

    // Issue a sibling GET to prove the H2 connection is still healthy
    // after the CONNECT stream teardown.
    let pseudo = PseudoHeaders {
        method: "GET".into(),
        scheme: "https".into(),
        authority: "example.com".into(),
        path: "/sibling".into(),
        protocol: None,
    };
    let resp = handle
        .send_request(pseudo, vec![("user-agent".into(), "x".into())], None)
        .await
        .expect("sibling request");
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body, b"sibling-ok");

    drop(handle);
    let (saw_end, saw_sib) = tokio::time::timeout(Duration::from_millis(1_000), server)
        .await
        .expect("server task did not finish in time")
        .expect("server task panicked");
    assert!(
        saw_end,
        "client never emitted END_STREAM on the CONNECT stream"
    );
    assert_eq!(saw_sib, Some(3), "sibling GET must use stream id 3");
}
