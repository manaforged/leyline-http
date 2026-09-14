#[path = "h2_support/mod.rs"]
mod support;

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use support::*;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf};
use tokio::sync::oneshot;
use tokio::time::timeout;

use leyline::h2::config::{H2Config, PseudoOrder, SettingId};
use leyline::h2::connection::{ClientConnection, PseudoHeaders};
use leyline::h2::frame::{FrameHeader, FrameType, HeadersFrame, PingFrame};

struct Counted {
    inner: DuplexStream,
    writes: Arc<AtomicUsize>,
}

impl AsyncRead for Counted {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for Counted {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let out = Pin::new(&mut self.inner).poll_write(cx, buf);
        if matches!(out, Poll::Ready(Ok(_))) {
            self.writes.fetch_add(1, Ordering::Relaxed);
        }
        out
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

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
        settings_flood_window: Duration::from_secs(10),
        header_block_reassembly_timeout: Duration::from_secs(10),
    }
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
async fn eight_concurrent_requests_share_one_write() {
    let (client_io, mut server_io) = tokio::io::duplex(256 * 1024);
    let writes = Arc::new(AtomicUsize::new(0));
    let counted = Counted {
        inner: client_io,
        writes: writes.clone(),
    };

    let (greeted_tx, greeted_rx) = oneshot::channel::<()>();
    let (headers_tx, headers_rx) = oneshot::channel::<()>();
    let (go_tx, go_rx) = oneshot::channel::<()>();

    let server = tokio::spawn(async move {
        read_preface(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        write_server_settings(&mut server_io).await;
        write_settings_ack(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Settings as u8);
        assert!(h.flags & 0x1 != 0, "expected SETTINGS ACK");
        greeted_tx.send(()).expect("greeted");

        let mut ids = Vec::new();
        while ids.len() < 8 {
            let (h, _) = read_frame(&mut server_io).await;
            if h.frame_type == FrameType::Headers as u8 {
                ids.push(h.stream_id);
            }
        }
        headers_tx.send(()).expect("headers seen");
        go_rx.await.expect("go");
        for id in ids {
            write_response(&mut server_io, id, b"ok").await;
        }
        server_io
    });

    let (handle, _driver) = ClientConnection::start(counted, test_config())
        .await
        .expect("handshake");
    greeted_rx.await.expect("greeted");
    let before = writes.load(Ordering::Relaxed);

    let reqs: Vec<_> = (0..8)
        .map(|i| {
            let (p, h) = get_req(&format!("/{i}"));
            handle.send_request(p, h, None)
        })
        .collect();
    let mut all = Box::pin(futures_util::future::join_all(reqs));
    let mut headers_rx = headers_rx;
    let mut during = None;
    let mut go = Some(go_tx);
    let responses = loop {
        tokio::select! {
            r = &mut all => break r,
            _ = &mut headers_rx, if during.is_none() => {
                during = Some(writes.load(Ordering::Relaxed) - before);
                if let Some(tx) = go.take() {
                    tx.send(()).expect("go");
                }
            }
        }
    };
    drop(all);
    let during = during.expect("server saw all eight header blocks");

    for r in responses {
        assert_eq!(r.expect("response").status, 200);
    }

    assert!(
        during <= 3,
        "request phase took {during} transport writes for 8 concurrent requests"
    );

    drop(handle);
    let _ = server.await;
}

#[tokio::test]
async fn buffered_pings_share_one_write() {
    let (client, mut peer) = tokio::io::duplex(65_536);
    let writes = Arc::new(AtomicUsize::new(0));
    let counted = Counted {
        inner: client,
        writes: Arc::clone(&writes),
    };
    let (handle, driver) = ClientConnection::start(counted, test_config())
        .await
        .expect("start client");
    timeout(Duration::from_secs(2), async {
        read_preface(&mut peer).await;
        let (initial, _) = read_frame(&mut peer).await;
        assert_eq!(initial.frame_type, FrameType::Settings as u8);
        write_server_settings(&mut peer).await;
        write_settings_ack(&mut peer).await;
        let (ack, _) = read_frame(&mut peer).await;
        assert_eq!(ack.frame_type, FrameType::Settings as u8);
        assert_eq!(ack.flags & 1, 1);
        let before = writes.load(Ordering::Relaxed);
        let mut frames = BytesMut::new();
        for value in 0..8_u64 {
            PingFrame {
                ack: false,
                payload: value.to_be_bytes(),
            }
            .encode(&mut frames);
        }
        peer.write_all(&frames).await.expect("write pings");
        for value in 0..8_u64 {
            let (ack, payload) = read_frame(&mut peer).await;
            assert_eq!(ack.frame_type, FrameType::Ping as u8);
            assert_eq!(ack.flags & 1, 1);
            assert_eq!(ack.stream_id, 0);
            assert_eq!(payload, value.to_be_bytes());
        }
        assert_eq!(writes.load(Ordering::Relaxed) - before, 1);
        drop(handle);
        driver.join().await.expect("driver");
    })
    .await
    .expect("acknowledgement deadline");
}

#[tokio::test]
async fn ping_ack_precedes_continuation_wait() {
    let (client, mut peer) = tokio::io::duplex(65_536);
    let (handle, driver) = ClientConnection::start(client, test_config())
        .await
        .expect("start client");
    let server = async move {
        read_preface(&mut peer).await;
        let (initial, _) = read_frame(&mut peer).await;
        assert_eq!(initial.frame_type, FrameType::Settings as u8);
        write_server_settings(&mut peer).await;
        write_settings_ack(&mut peer).await;
        let (ack, _) = read_frame(&mut peer).await;
        assert_eq!(ack.frame_type, FrameType::Settings as u8);
        assert_eq!(ack.flags & 1, 1);
        let (request, _) = read_frame(&mut peer).await;
        assert_eq!(request.frame_type, FrameType::Headers as u8);
        let payload = 73_u64.to_be_bytes();
        let mut frames = BytesMut::new();
        PingFrame {
            ack: false,
            payload,
        }
        .encode(&mut frames);
        HeadersFrame {
            stream_id: request.stream_id,
            end_stream: true,
            end_headers: false,
            priority: None,
            fragment: Bytes::from_static(&[0x88]),
        }
        .encode(&mut frames);
        peer.write_all(&frames)
            .await
            .expect("write ping and headers");
        let (ack, received) = timeout(Duration::from_millis(200), read_frame(&mut peer))
            .await
            .expect("ping acknowledgement before continuation");
        assert_eq!(ack.frame_type, FrameType::Ping as u8);
        assert_eq!(ack.flags & 1, 1);
        assert_eq!(ack.stream_id, 0);
        assert_eq!(received, payload);
        frames.clear();
        FrameHeader {
            length: 0,
            frame_type: FrameType::Continuation as u8,
            flags: 4,
            stream_id: request.stream_id,
        }
        .encode(&mut frames);
        peer.write_all(&frames).await.expect("write continuation");
        peer
    };
    let (pseudo, headers) = get_req("/continued");
    let (peer, response) = timeout(Duration::from_secs(2), async {
        tokio::join!(server, handle.send_request(pseudo, headers, None))
    })
    .await
    .expect("request deadline");
    let response = response.expect("response");
    assert_eq!(response.status, 200);
    assert!(response.body.is_empty());
    drop(handle);
    timeout(Duration::from_secs(2), driver.join())
        .await
        .expect("driver deadline")
        .expect("driver");
    drop(peer);
}
