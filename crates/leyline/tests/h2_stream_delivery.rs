#[path = "h2_support/mod.rs"]
mod support;

use std::io;
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use leyline::h2::connection::PseudoHeaders;
use leyline::h2::frame::{FrameType, PingFrame, RstStreamFrame};
use leyline::h2::{ErrorCode, H2Client, H2Config, Head, RequestBody, ResponseBody};
use leyline::profile::BrowserProfile;
use support::{
    read_frame, read_preface, write_data, write_raw_headers, write_response_headers,
    write_server_settings, write_settings_ack,
};
use tokio::io::{AsyncWriteExt, DuplexStream};
use tokio::sync::oneshot;
use tokio::time::timeout;

const SIZE: usize = 96 * 16_384;

#[derive(Clone, Copy)]
enum Ending {
    Complete,
    Trailers,
    Reset,
    Disconnect,
}

async fn barrier(peer: &mut DuplexStream) {
    let ping = PingFrame {
        ack: false,
        payload: *b"delivery",
    };
    let mut encoded = BytesMut::new();
    ping.encode(&mut encoded);
    peer.write_all(&encoded).await.expect("write PING");
    loop {
        let (header, payload) = read_frame(peer).await;
        if header.frame_type == FrameType::Ping as u8 {
            assert_eq!(header.flags & 1, 1);
            assert_eq!(payload, b"delivery");
            return;
        }
    }
}

async fn closed(handle: &H2Client) {
    timeout(Duration::from_secs(3), async {
        while !handle.is_closed() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("connection closes");
}

async fn transfer(ending: Ending) -> (usize, Option<io::Error>) {
    let expected: Arc<[u8]> = (0..SIZE).map(|i| (i % 251) as u8).collect();
    let payload = Arc::clone(&expected);
    let config = H2Config::from_profile(&BrowserProfile::bare().h2).expect("bare H2 config");
    let (client, mut peer) = tokio::io::duplex(1 << 20);
    let (ready_tx, ready_rx) = oneshot::channel();
    let (done_tx, done_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        read_preface(&mut peer).await;
        let (header, _) = read_frame(&mut peer).await;
        assert_eq!(header.frame_type, FrameType::Settings as u8);
        write_server_settings(&mut peer).await;
        write_settings_ack(&mut peer).await;
        let stream_id = loop {
            let (header, _) = read_frame(&mut peer).await;
            if header.frame_type == FrameType::Headers as u8 {
                break header.stream_id;
            }
        };
        write_response_headers(&mut peer, stream_id).await;
        for (index, chunk) in payload.chunks(16_384).enumerate() {
            let end = matches!(ending, Ending::Complete) && (index + 1) * 16_384 == SIZE;
            write_data(&mut peer, stream_id, chunk, end).await;
        }
        if matches!(ending, Ending::Trailers) {
            write_raw_headers(&mut peer, stream_id, &[("x-complete", "yes")], true).await;
        }
        if matches!(ending, Ending::Reset) {
            let reset = RstStreamFrame {
                stream_id,
                error_code: ErrorCode::Cancel,
            };
            let mut encoded = BytesMut::new();
            reset.encode(&mut encoded);
            peer.write_all(&encoded).await.expect("RST_STREAM");
        }
        barrier(&mut peer).await;
        if matches!(
            ending,
            Ending::Complete | Ending::Trailers | Ending::Disconnect
        ) {
            drop(peer);
            ready_tx.send(()).expect("consumer waiting");
        } else {
            ready_tx.send(()).expect("consumer waiting");
            done_rx.await.expect("consumer finished");
        }
    });
    let handle = leyline::h2::start(client, config)
        .await
        .expect("H2 connection");
    let response = handle
        .send_shared(
            Arc::new(Head {
                pseudo: PseudoHeaders {
                    method: "GET".into(),
                    scheme: "https".into(),
                    authority: "example.test".into(),
                    path: "/".into(),
                    protocol: None,
                },
                headers: Vec::new(),
            }),
            RequestBody::None,
            true,
        )
        .await
        .expect("response headers");
    assert_eq!(response.status, 200);
    let ResponseBody::Streaming(mut body) = response.body else {
        panic!("streaming body");
    };
    timeout(Duration::from_secs(3), ready_rx)
        .await
        .expect("peer completed frames")
        .expect("peer barrier");
    if !matches!(ending, Ending::Reset) {
        closed(&handle).await;
    }
    let mut received = 0;
    let mut failure = None;
    while let Some(chunk) = timeout(Duration::from_secs(3), body.recv())
        .await
        .expect("body delivery")
    {
        match chunk {
            Ok(chunk) => {
                let end = received + chunk.len();
                assert!(end <= expected.len());
                assert!(chunk.as_ref() == &expected[received..end]);
                received = end;
            }
            Err(error) => {
                failure = Some(error);
                assert!(
                    timeout(Duration::from_secs(3), body.recv())
                        .await
                        .expect("error is terminal")
                        .is_none()
                );
                break;
            }
        }
    }
    if matches!(ending, Ending::Reset) {
        done_tx.send(()).expect("server waiting");
    }
    server.await.expect("peer task");
    closed(&handle).await;
    (received, failure)
}

#[tokio::test]
async fn completed_body_survives_connection_close() {
    let (received, failure) = transfer(Ending::Complete).await;
    assert!(failure.is_none(), "{failure:?}");
    assert_eq!(received, SIZE);
}

#[tokio::test]
async fn peer_eof_is_not_success_without_end_stream() {
    let (_, failure) = transfer(Ending::Disconnect).await;
    assert!(failure.is_some(), "incomplete response returned EOF");
}

#[tokio::test]
async fn stream_reset_survives_a_full_body_queue() {
    let (_, failure) = transfer(Ending::Reset).await;
    let failure = failure.expect("stream reset");
    assert!(failure.to_string().contains("Cancel"), "{failure}");
}

#[tokio::test]
async fn trailers_preserve_queued_body_data() {
    let (received, failure) = transfer(Ending::Trailers).await;
    assert!(failure.is_none(), "{failure:?}");
    assert_eq!(received, SIZE);
}
