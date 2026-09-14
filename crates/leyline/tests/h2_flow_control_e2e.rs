#[path = "h2_support/mod.rs"]
mod support;

use std::time::Duration;

use bytes::BytesMut;
use leyline::h2::config::{H2Config, PseudoOrder, SettingId};
use leyline::h2::connection::{ClientConnection, PseudoHeaders};
use leyline::h2::frame::{DataFrame, FrameType, HeadersFrame, PingFrame};
use leyline::h2::hpack;
use leyline::h2::{RequestBody, ResponseBody};
use leyline::profile::BrowserProfile;
use support::*;
use tokio::io::AsyncWriteExt;
use tokio::time::{sleep, timeout};

const STREAM_WINDOW: u32 = 65_535;
const CONN_WINDOW: u32 = 1_048_576;
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
        max_response_body_bytes: 100 * 1024 * 1024,
        max_header_block_bytes: 256 * 1024,
        settings_flood_threshold: 100,
        settings_flood_window: Duration::from_secs(10),
        header_block_reassembly_timeout: Duration::from_secs(10),
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
    transfer(window_config(), BODY_LEN, false).await;
}

#[tokio::test]
async fn bare_profile_receives_beyond_its_initial_windows() {
    let config = H2Config::from_profile(&BrowserProfile::bare().h2).expect("bare H2 config");
    transfer(config, 8 * 1024 * 1024, true).await;
}

async fn transfer(config: H2Config, body_len: usize, bulk: bool) {
    let (client_io, mut server_io) = tokio::io::duplex(1 << 20);

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        let (h, payload) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8, "client SETTINGS");
        let window = payload
            .as_chunks::<6>()
            .0
            .iter()
            .find(|setting| setting[..2] == [0, 4])
            .map(|setting| u32::from_be_bytes(setting[2..].try_into().expect("setting value")))
            .expect("initial stream window");
        if bulk {
            assert!(window >= 1024 * 1024);
        }
        write_server_settings_with(&mut server_io, vec![(0x4, 65_535)]).await;
        write_settings_ack(&mut server_io).await;

        let mut stream_credit = u64::from(window);
        let mut conn_credit: u64 = 65_535;

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
                _ => {}
            }
        };

        if bulk {
            assert!(conn_credit >= 1024 * 1024);
        }
        write_response_headers(&mut server_io, stream_id).await;

        let mut sent: usize = 0;
        while sent < body_len {
            let budget = (body_len - sent)
                .min(MAX_FRAME)
                .min(stream_credit as usize)
                .min(conn_credit as usize);
            if budget == 0 {
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

            let end_stream = sent + budget == body_len;
            let d = DataFrame {
                stream_id,
                end_stream,
                data: bytes::Bytes::from(vec![0xAB; budget]),
                wire_len: budget as u64,
            };
            let mut buf = BytesMut::new();
            d.encode(&mut buf);
            server_io.write_all(&buf).await.expect("data write");
            sent += budget;
            stream_credit -= budget as u64;
            conn_credit -= budget as u64;

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

    let (handle, _driver) = ClientConnection::start(client_io, config)
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
    assert_eq!(resp.body.len(), body_len);
    assert!(resp.body.iter().all(|byte| *byte == 0xAB));

    server.await.expect("mock server panicked");
}

#[tokio::test]
async fn bare_stream_withholds_credit_until_consumed() {
    backpressure(false).await;
}

#[tokio::test]
async fn stalled_response_outlives_last_handle() {
    backpressure(true).await;
}

async fn backpressure(orphaned: bool) {
    let config = H2Config::from_profile(&BrowserProfile::bare().h2).expect("bare H2 config");
    let window = config
        .settings
        .iter()
        .find(|(id, _)| *id == SettingId::InitialWindowSize)
        .expect("stream window")
        .1 as usize;
    let (client_io, mut server_io) = tokio::io::duplex(1 << 20);
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        let (header, _) = read_frame(&mut server_io).await;
        assert_eq!(header.frame_type, FrameType::Settings as u8);
        write_server_settings(&mut server_io).await;
        write_settings_ack(&mut server_io).await;
        let stream_id = loop {
            let (header, _) = read_frame(&mut server_io).await;
            if header.frame_type == FrameType::Headers as u8 {
                break header.stream_id;
            }
        };
        write_response_headers(&mut server_io, stream_id).await;
        for offset in (0..window).step_by(MAX_FRAME) {
            write_data(
                &mut server_io,
                stream_id,
                &vec![0xAB; (window - offset).min(MAX_FRAME)],
                false,
            )
            .await;
        }
        let ping = PingFrame {
            ack: false,
            payload: *b"capacity",
        };
        let mut encoded = BytesMut::new();
        ping.encode(&mut encoded);
        server_io.write_all(&encoded).await.expect("PING");
        loop {
            let (header, payload) = read_frame(&mut server_io).await;
            if header.frame_type == FrameType::WindowUpdate as u8 {
                assert_eq!(header.stream_id, 0, "stalled stream received more credit");
            }
            if header.frame_type == FrameType::Ping as u8 {
                assert_eq!(header.flags & 1, 1);
                assert_eq!(payload, b"capacity");
                break;
            }
        }
        ready_tx.send(()).expect("consumer waiting");
        loop {
            let (header, payload) = read_frame(&mut server_io).await;
            if header.frame_type == FrameType::WindowUpdate as u8 && header.stream_id == stream_id {
                let increment = u32::from_be_bytes(payload.try_into().expect("window increment"));
                assert!(increment > 0);
                break;
            }
        }
        write_data(&mut server_io, stream_id, &[0xAB], true).await;
        done_rx.await.expect("body consumed");
        if orphaned {
            loop {
                let (header, payload) = read_frame(&mut server_io).await;
                if header.frame_type == FrameType::GoAway as u8 {
                    assert_eq!(
                        u32::from_be_bytes(payload[..4].try_into().expect("last peer stream")),
                        0
                    );
                    break;
                }
            }
        }
    });
    let (handle, driver) = ClientConnection::start(client_io, config)
        .await
        .expect("H2 connection");
    let response = handle
        .send_request_ex(
            PseudoHeaders {
                method: "GET".into(),
                scheme: "https".into(),
                authority: "example.test".into(),
                path: "/".into(),
                protocol: None,
            },
            Vec::new(),
            RequestBody::None,
            true,
        )
        .await
        .expect("streaming response");
    assert_eq!(response.status, 200);
    let ResponseBody::Streaming(mut body) = response.body else {
        panic!("streaming body");
    };
    timeout(Duration::from_secs(3), ready_rx)
        .await
        .expect("server reached the receive limit")
        .expect("server credit check");
    if orphaned {
        drop(handle);
        sleep(Duration::from_millis(750)).await;
    }
    let mut received = Vec::new();
    while let Some(chunk) = timeout(Duration::from_secs(3), body.recv())
        .await
        .expect("body resumes after consumption")
    {
        received.extend_from_slice(&chunk.expect("body chunk"));
    }
    assert_eq!(received.len(), window + 1);
    assert!(received.iter().all(|byte| *byte == 0xAB));
    done_tx.send(()).expect("server waiting");
    server.await.expect("server");
    if orphaned {
        timeout(Duration::from_secs(3), driver.join())
            .await
            .expect("driver exits")
            .expect("driver shutdown");
    }
}
