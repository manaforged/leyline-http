#[path = "h2_support/mod.rs"]
mod support;

use std::sync::Arc;
use std::time::Duration;

use leyline::h2::RequestBody;
use leyline::h2::frame::FrameType;
use support::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn handshake<S: AsyncReadExt + AsyncWriteExt + Unpin>(server_io: &mut S) {
    read_preface(server_io).await;
    let (h, _) = read_frame(server_io).await;
    assert_eq!(h.frame_type, FrameType::Settings as u8);
    write_server_settings(server_io).await;
    write_settings_ack(server_io).await;
    let (h, _) = read_frame(server_io).await;
    assert_eq!(h.frame_type, FrameType::Settings as u8);
    assert!(h.flags & 0x1 != 0, "expected client SETTINGS ack");
}

async fn write_goaway<S: AsyncWriteExt + Unpin>(server_io: &mut S, last_stream_id: u32, code: u32) {
    let mut frame = vec![0, 0, 8, FrameType::GoAway as u8, 0, 0, 0, 0, 0];
    frame.extend_from_slice(&last_stream_id.to_be_bytes());
    frame.extend_from_slice(&code.to_be_bytes());
    server_io.write_all(&frame).await.expect("goaway write");
}

#[tokio::test]
async fn goaway_no_error_refuses_streams_above_last_id() {
    let (client_io, mut server_io) = tokio::io::duplex(65_536);

    let server = tokio::spawn(async move {
        handshake(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Headers as u8);
        assert_eq!(h.stream_id, 1);
        write_goaway(&mut server_io, 0, 0).await;
        let mut sink = [0u8; 256];
        drop(server_io.read(&mut sink).await);
    });

    let handle = leyline::h2::start(client_io, test_config())
        .await
        .expect("handshake");
    let head = Arc::new(get_head("/"));
    let err = handle
        .send_shared(head, RequestBody::None, false)
        .await
        .expect_err("refused");
    assert!(
        matches!(
            err,
            leyline::h2::H2Error::Stream {
                stream_id: 1,
                code: leyline::h2::ErrorCode::RefusedStream
            }
        ),
        "expected RefusedStream, got {err:?}"
    );
    server.abort();
}

#[tokio::test]
async fn slow_streaming_consumer_is_not_cancelled() {
    let (client_io, mut server_io) = tokio::io::duplex(1 << 20);
    const CHUNKS: usize = 200;
    const CHUNK: usize = 100;

    let server = tokio::spawn(async move {
        handshake(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Headers as u8);
        write_raw_headers(&mut server_io, 1, &[(":status", "200")], false).await;
        for i in 0..CHUNKS {
            write_data(&mut server_io, 1, &[b'a'; CHUNK], i + 1 == CHUNKS).await;
        }
        loop {
            let mut sink = [0u8; 256];
            match server_io.read(&mut sink).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    });

    let handle = leyline::h2::start(client_io, test_config())
        .await
        .expect("handshake");
    let head = Arc::new(get_head("/"));
    let resp = handle
        .send_shared(head, RequestBody::None, true)
        .await
        .expect("head");
    let leyline::h2::ResponseBody::Streaming(mut rx) = resp.body else {
        panic!("expected a streaming body");
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut total = 0usize;
    while let Some(chunk) = rx.recv().await {
        total += chunk.expect("chunk").len();
    }
    assert_eq!(total, CHUNKS * CHUNK);
    server.abort();
}

#[tokio::test]
async fn early_end_stream_resets_open_request_body() {
    let (client_io, mut server_io) = tokio::io::duplex(65_536);

    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel::<u32>();
    let server = tokio::spawn(async move {
        handshake(&mut server_io).await;
        let (h, _) = read_frame(&mut server_io).await;
        assert_eq!(h.frame_type, FrameType::Headers as u8);
        write_raw_headers(&mut server_io, 1, &[(":status", "200")], true).await;
        let mut seen_tx = Some(seen_tx);
        loop {
            let (h, payload) = read_frame(&mut server_io).await;
            if h.frame_type == FrameType::RstStream as u8 && h.stream_id == 1 {
                let code = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                if let Some(tx) = seen_tx.take() {
                    tx.send(code)
                        .expect("the RST_STREAM listener is still waiting");
                }
                break;
            }
        }
    });

    let handle = leyline::h2::start(client_io, test_config())
        .await
        .expect("handshake");
    let mut head = get_head("/");
    head.pseudo.method = "POST".into();
    let (body_tx, body_rx) = tokio::sync::mpsc::channel::<std::io::Result<bytes::Bytes>>(4);
    body_tx
        .send(Ok(bytes::Bytes::from_static(b"first")))
        .await
        .expect("queue");
    let stream = futures_util::stream::unfold(body_rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    });
    let resp = handle
        .send_shared(
            Arc::new(head),
            RequestBody::Streaming {
                stream: Box::pin(stream),
                length_hint: None,
            },
            false,
        )
        .await
        .expect("response");
    assert_eq!(resp.status, 200);
    let code = tokio::time::timeout(Duration::from_secs(2), seen_rx)
        .await
        .expect("client sent RST_STREAM")
        .expect("server task alive");
    assert_eq!(code, 0, "expected NO_ERROR");
    drop(body_tx);
    server.abort();
}
