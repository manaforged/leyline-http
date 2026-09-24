#[path = "h2_support/mod.rs"]
mod support;

use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use futures_util::stream::empty;
use support::*;
use tokio::io::AsyncReadExt;
use tokio::sync::oneshot;
use tokio::time::timeout;

use leyline::h2::config::{H2Config, PseudoOrder, SettingId};
use leyline::h2::connection::PseudoHeaders;
use leyline::h2::error::{ErrorCode, H2Error};
use leyline::h2::frame::{FrameType, GoAwayFrame};
use leyline::h2::{Head, RequestBody, ResponseBody};

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
        max_response_body_bytes: 100 * 1024 * 1024,
        max_header_block_bytes: 256 * 1024,
        settings_flood_threshold: 100,
        settings_flood_window: std::time::Duration::from_secs(10),
        header_block_reassembly_timeout: std::time::Duration::from_secs(10),
    }
}

fn head(
    pseudo: PseudoHeaders,
    headers: Vec<(
        std::borrow::Cow<'static, str>,
        std::borrow::Cow<'static, str>,
    )>,
) -> Arc<Head> {
    Arc::new(Head { pseudo, headers })
}

fn buffered(body: ResponseBody) -> Vec<u8> {
    let ResponseBody::Buffered(body) = body else {
        panic!("buffered body")
    };
    body
}

#[allow(clippy::type_complexity)]
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

#[tokio::test]
async fn two_concurrent_requests_respond_out_of_order() {
    let (client_io, mut server_io) = tokio::io::duplex(65_536);

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        write_server_settings(&mut server_io).await;
        write_settings_ack(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        assert!(h.flags & 0x1 != 0, "expected SETTINGS ACK");

        let (h1, _) = read_frame(&mut server_io).await;
        assert_eq!(h1.frame_type, FrameType::Headers as u8);
        assert_eq!(h1.stream_id, 1);
        let (h2, _) = read_frame(&mut server_io).await;
        assert_eq!(h2.frame_type, FrameType::Headers as u8);
        assert_eq!(h2.stream_id, 3);

        write_response(&mut server_io, 3, b"second-response").await;
        write_response(&mut server_io, 1, b"first-response").await;

        let mut sink = [0u8; 256];
        let _ = server_io.read(&mut sink).await;
    });

    let handle = leyline::h2::start(client_io, test_config())
        .await
        .expect("handshake");

    let (p1, h1) = get_req("/one");
    let (p2, h2) = get_req("/two");

    let h1_clone = handle.clone();
    let h2_clone = handle.clone();
    let fut_a = tokio::spawn(async move {
        h1_clone
            .send_shared(head(p1, h1), RequestBody::None, false)
            .await
    });
    let fut_b = tokio::spawn(async move {
        h2_clone
            .send_shared(head(p2, h2), RequestBody::None, false)
            .await
    });

    let (a, b) = tokio::join!(fut_a, fut_b);
    let a = a.unwrap().expect("req a");
    let b = b.unwrap().expect("req b");
    assert_eq!(a.status, 200);
    assert_eq!(b.status, 200);
    assert_eq!(buffered(a.body), b"first-response");
    assert_eq!(buffered(b.body), b"second-response");

    drop(handle);
    let _ = server.await;
}

#[tokio::test]
async fn parked_stream_does_not_block_others() {
    let (client_io, mut server_io) = tokio::io::duplex(65_536);

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;

        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);

        write_server_settings_with(&mut server_io, vec![(0x4, 10)]).await;

        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        assert!(h.flags & 0x1 != 0);

        write_settings_ack(&mut server_io).await;

        let (ha, _) = read_frame(&mut server_io).await;
        assert_eq!(ha.frame_type, FrameType::Headers as u8);
        assert_eq!(ha.stream_id, 1);
        assert!(ha.flags & 0x1 == 0, "A HEADERS must not END_STREAM");

        let mut got_a_data_first = false;
        let (maybe_data_or_headers, p) = read_frame(&mut server_io).await;
        if maybe_data_or_headers.frame_type == FrameType::Data as u8
            && maybe_data_or_headers.stream_id == 1
        {
            got_a_data_first = true;
            assert_eq!(p.len(), 10);
            assert!(maybe_data_or_headers.flags & 0x1 == 0);
            let (hb, _) = read_frame(&mut server_io).await;
            assert_eq!(hb.frame_type, FrameType::Headers as u8);
            assert_eq!(hb.stream_id, 3);
        } else {
            assert_eq!(maybe_data_or_headers.frame_type, FrameType::Headers as u8);
            assert_eq!(maybe_data_or_headers.stream_id, 3);
        }

        let (nf, np) = read_frame(&mut server_io).await;
        if got_a_data_first {
            assert_eq!(nf.frame_type, FrameType::Data as u8);
            assert_eq!(nf.stream_id, 3);
            assert_eq!(np.len(), 5);
            assert!(nf.flags & 0x1 != 0, "B DATA must END_STREAM");
        } else {
            assert_eq!(nf.frame_type, FrameType::Data as u8);
            assert_eq!(nf.stream_id, 1);
            assert_eq!(np.len(), 10);
            assert!(nf.flags & 0x1 == 0);
            let (bf, bp) = read_frame(&mut server_io).await;
            assert_eq!(bf.frame_type, FrameType::Data as u8);
            assert_eq!(bf.stream_id, 3);
            assert_eq!(bp.len(), 5);
            assert!(bf.flags & 0x1 != 0);
        }

        write_response(&mut server_io, 3, b"bbb").await;

        write_window_update(&mut server_io, 0, 64).await;
        write_window_update(&mut server_io, 1, 20).await;

        let (af, ap) = read_frame(&mut server_io).await;
        assert_eq!(af.frame_type, FrameType::Data as u8);
        assert_eq!(af.stream_id, 1);
        assert_eq!(ap.len(), 10);
        assert!(af.flags & 0x1 != 0, "A final DATA must END_STREAM");

        write_response(&mut server_io, 1, b"aaa").await;

        let mut sink = [0u8; 256];
        let _ = server_io.read(&mut sink).await;
    });

    let handle = leyline::h2::start(client_io, test_config())
        .await
        .expect("handshake");

    let (p_a, h_a) = get_req("/a");
    let (p_b, h_b) = get_req("/b");

    let body_a = bytes::Bytes::from_static(&[b'A'; 20]);
    let body_b = bytes::Bytes::from_static(&[b'B'; 5]);

    let ha = handle.clone();
    let hb = handle.clone();
    let fut_a = tokio::spawn(async move {
        ha.send_shared(
            head(
                PseudoHeaders {
                    method: "POST".into(),
                    ..p_a
                },
                h_a,
            ),
            RequestBody::from(Some(body_a)),
            false,
        )
        .await
    });
    let fut_b = tokio::spawn(async move {
        hb.send_shared(
            head(
                PseudoHeaders {
                    method: "POST".into(),
                    ..p_b
                },
                h_b,
            ),
            RequestBody::from(Some(body_b)),
            false,
        )
        .await
    });

    let b_result = tokio::time::timeout(Duration::from_secs(3), fut_b)
        .await
        .expect("B timeout");
    let b = b_result.unwrap().expect("req B");
    assert_eq!(b.status, 200);
    assert_eq!(buffered(b.body), b"bbb");

    let a_result = tokio::time::timeout(Duration::from_secs(3), fut_a)
        .await
        .expect("A timeout");
    let a = a_result.unwrap().expect("req A");
    assert_eq!(a.status, 200);
    assert_eq!(buffered(a.body), b"aaa");

    drop(handle);
    let _ = server.await;
}

#[tokio::test]
async fn driver_shuts_down_on_last_handle_drop() {
    let (client_io, mut server_io) = tokio::io::duplex(4096);

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        let (_h, _) = read_frame(&mut server_io).await;
        write_server_settings(&mut server_io).await;
        write_settings_ack(&mut server_io).await;
        let (_h, _) = read_frame(&mut server_io).await;
        let (hdr, payload) = read_frame(&mut server_io).await;
        assert_eq!(hdr.frame_type, FrameType::GoAway as u8);
        let g = GoAwayFrame::parse(hdr, bytes::Bytes::from(payload)).unwrap();
        assert_eq!(g.error_code as u32, 0, "NO_ERROR goaway expected");
    });

    let handle = leyline::h2::start(client_io, test_config())
        .await
        .expect("handshake");

    let clone1 = handle.clone();
    let clone2 = handle.clone();
    drop(handle);
    drop(clone1);
    drop(clone2);

    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("goaway timeout")
        .expect("server");
}

#[tokio::test]
async fn reader_eof_fails_pending_requests() {
    let (client_io, mut server_io) = tokio::io::duplex(4096);

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        let (_h, _) = read_frame(&mut server_io).await;
        write_server_settings(&mut server_io).await;
        write_settings_ack(&mut server_io).await;
        let (_h, _) = read_frame(&mut server_io).await;
        let (_a, _) = read_frame(&mut server_io).await;
        let (_b, _) = read_frame(&mut server_io).await;

        drop(server_io);
    });

    let handle = leyline::h2::start(client_io, test_config())
        .await
        .expect("handshake");

    let (p1, h1) = get_req("/one");
    let (p2, h2) = get_req("/two");

    let ha = handle.clone();
    let hb = handle.clone();
    let fut_a =
        tokio::spawn(async move { ha.send_shared(head(p1, h1), RequestBody::None, false).await });
    let fut_b =
        tokio::spawn(async move { hb.send_shared(head(p2, h2), RequestBody::None, false).await });

    let (a, b) = tokio::time::timeout(Duration::from_secs(3), async { tokio::join!(fut_a, fut_b) })
        .await
        .expect("joins timeout");

    assert!(a.unwrap().is_err(), "req A must fail on EOF");
    assert!(b.unwrap().is_err(), "req B must fail on EOF");

    let _ = tokio::time::timeout(Duration::from_secs(1), server).await;
    let _ = BytesMut::new();
}

#[tokio::test]
async fn zero_stream_limit_waits_for_peer_update() {
    timeout(Duration::from_secs(5), async {
        let (client_io, mut server_io) = tokio::io::duplex(65_536);
        let (release, ready) = oneshot::channel();
        let (greeted, greeting) = oneshot::channel();
        let (done, finished) = oneshot::channel();
        let server = tokio::spawn(async move {
            read_preface(&mut server_io).await;
            read_frame(&mut server_io).await;
            write_server_settings_with(&mut server_io, vec![(0x3, 0)]).await;
            write_settings_ack(&mut server_io).await;
            let (ack, _) = read_frame(&mut server_io).await;
            assert_eq!(ack.frame_type, FrameType::Settings as u8);
            assert_eq!(ack.flags, 1);
            greeted.send(()).expect("peer settings acknowledged");
            ready.await.expect("response release");
            write_server_settings_with(&mut server_io, vec![(0x3, 1)]).await;
            let request = loop {
                let (frame, _) = read_frame(&mut server_io).await;
                if frame.frame_type == FrameType::Headers as u8 {
                    break frame;
                }
            };
            assert_eq!(request.stream_id, 1);
            write_response(&mut server_io, request.stream_id, b"resumed").await;
            finished.await.expect("client assertions complete");
        });
        let handle = leyline::h2::start(client_io, test_config())
            .await
            .expect("response or connection");
        greeting.await.expect("peer settings applied");
        let (pseudo, headers) = get_req("/blocked");
        let error = handle
            .send_shared(
                head(pseudo, headers),
                RequestBody::Streaming {
                    stream: Box::pin(empty()),
                    length_hint: None,
                },
                false,
            )
            .await
            .expect_err("peer capacity exhausted");
        assert!(matches!(
            error,
            H2Error::Connection {
                code: ErrorCode::RefusedStream,
                ..
            }
        ));
        release.send(()).expect("release server");
        let (pseudo, headers) = get_req("/resumed");
        let response = handle
            .send_shared(head(pseudo, headers), RequestBody::None, false)
            .await
            .expect("response");
        assert_eq!(response.status, 200);
        assert_eq!(buffered(response.body), b"resumed");
        done.send(()).expect("complete server");
        server.await.expect("server task");
    })
    .await
    .expect("stream limit update stalled");
}

#[tokio::test]
async fn closed_stream_with_unread_chunks_releases_peer_capacity() {
    timeout(Duration::from_secs(5), async {
        let (client_io, mut server_io) = tokio::io::duplex(65_536);
        let (release, ready) = oneshot::channel();
        let (done, finished) = oneshot::channel();
        let server = tokio::spawn(async move {
            read_preface(&mut server_io).await;
            read_frame(&mut server_io).await;
            write_server_settings_with(&mut server_io, vec![(0x3, 1)]).await;
            write_settings_ack(&mut server_io).await;
            loop {
                let (frame, _) = read_frame(&mut server_io).await;
                if frame.frame_type == FrameType::Headers as u8 {
                    assert_eq!(frame.stream_id, 1);
                    break;
                }
            }
            write_response_headers(&mut server_io, 1).await;
            ready.await.expect("response release");
            for byte in 0..40u8 {
                write_data(&mut server_io, 1, &[byte], byte == 39).await;
            }
            loop {
                let (frame, _) = read_frame(&mut server_io).await;
                if frame.frame_type == FrameType::Headers as u8 {
                    assert_eq!(frame.stream_id, 3);
                    write_response(&mut server_io, 3, b"next").await;
                    break;
                }
            }
            finished.await.expect("client assertions complete");
        });
        let handle = leyline::h2::start(client_io, test_config())
            .await
            .expect("response or connection");
        let (pseudo, headers) = get_req("/stream");
        let response = handle
            .send_shared(head(pseudo, headers), RequestBody::None, true)
            .await
            .expect("response or connection");
        assert_eq!(response.status, 200);
        let ResponseBody::Streaming(mut body) = response.body else {
            panic!("expected streaming body");
        };
        let (pseudo, headers) = get_req("/blocked");
        let error = handle
            .send_shared(
                head(pseudo, headers),
                RequestBody::Streaming {
                    stream: Box::pin(empty()),
                    length_hint: None,
                },
                false,
            )
            .await
            .expect_err("peer capacity exhausted");
        assert!(matches!(
            error,
            H2Error::Connection {
                code: ErrorCode::RefusedStream,
                ..
            }
        ));
        release.send(()).expect("release server");
        let (pseudo, headers) = get_req("/next");
        let response = handle
            .send_shared(head(pseudo, headers), RequestBody::None, false)
            .await
            .expect("response");
        assert_eq!(response.status, 200);
        assert_eq!(buffered(response.body), b"next");
        let mut received = Vec::new();
        while let Some(chunk) = body.recv().await {
            received.extend_from_slice(&chunk.expect("response chunk"));
        }
        assert_eq!(received, (0..40u8).collect::<Vec<_>>());
        done.send(()).expect("complete server");
        server.await.expect("server task");
    })
    .await
    .expect("closed response retained peer capacity");
}

#[tokio::test]
async fn queued_request_uses_acknowledged_settings() {
    let (client, mut peer) = tokio::io::duplex(65_536);
    let (ready, proceed) = oneshot::channel();
    let server = tokio::spawn(async move {
        read_preface(&mut peer).await;
        let (initial, _) = read_frame(&mut peer).await;
        assert_eq!(initial.frame_type, FrameType::Settings as u8);
        proceed.await.expect("release peer settings");
        write_server_settings_with(&mut peer, vec![(0x1, 0)]).await;
        write_settings_ack(&mut peer).await;
        let (ack, payload) = read_frame(&mut peer).await;
        assert_eq!(ack.frame_type, FrameType::Settings as u8);
        assert_eq!(ack.flags & 0x1, 0x1);
        assert!(payload.is_empty());
        let (request, payload) = read_frame(&mut peer).await;
        assert_eq!(request.frame_type, FrameType::Headers as u8);
        assert_eq!(payload.first(), Some(&0x20));
        write_response(&mut peer, request.stream_id, b"configured").await;
        let mut closed = [0; 9];
        let _ = peer.read(&mut closed).await;
    });
    let handle = leyline::h2::start(client, test_config())
        .await
        .expect("start client");
    let (pseudo, headers) = get_req("/");
    let mut request = Box::pin(handle.send_shared(head(pseudo, headers), RequestBody::None, false));
    assert!(
        timeout(Duration::from_millis(10), request.as_mut())
            .await
            .is_err()
    );
    ready.send(()).expect("release settings");
    let response = timeout(Duration::from_secs(2), request.as_mut())
        .await
        .expect("response deadline")
        .expect("response");
    assert_eq!(response.status, 200);
    assert_eq!(buffered(response.body), b"configured");
    drop(request);
    drop(handle);
    timeout(Duration::from_secs(2), server)
        .await
        .expect("server deadline")
        .expect("server");
}
