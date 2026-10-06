#[path = "h2_support/mod.rs"]
mod support;

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use futures_util::Stream;
use support::*;
use tokio::io::{AsyncReadExt, DuplexStream};
use tokio::time::timeout;

use leyline::h2::connection::PseudoHeaders;
use leyline::h2::frame::{FRAME_HEADER_LEN, FrameHeader, FrameType};
use leyline::h2::{H2Client, Head, RequestBody};

#[tokio::test]
async fn a_peer_that_stops_reading_still_delivers_its_response() {
    const INITIAL_WINDOW_SIZE: u16 = 0x4;
    const WIDE: u32 = 1 << 30;
    let (client_io, mut server_io) = tokio::io::duplex(64 * 1024);
    let handle = leyline::h2::start(client_io, test_config())
        .await
        .expect("handshake");
    read_preface(&mut server_io).await;
    read_frame(&mut server_io).await;
    write_server_settings_with(&mut server_io, vec![(INITIAL_WINDOW_SIZE, WIDE)]).await;
    write_settings_ack(&mut server_io).await;
    write_window_update(&mut server_io, 0, WIDE).await;

    let producer = Arc::new(Producer::default());
    let client = handle.clone();
    let upload = tokio::spawn(async move {
        client
            .send_shared(head("POST"), endless(&producer), false)
            .await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    write_raw_headers(&mut server_io, 1, &[(":status", "413")], true).await;

    let response = timeout(Duration::from_secs(3), upload)
        .await
        .expect("the response arrives while the upload is blocked")
        .unwrap()
        .unwrap();
    assert_eq!(response.status, 413);
}

async fn wide_open(buffer: usize, max_frame: u32) -> (H2Client, DuplexStream) {
    const INITIAL_WINDOW_SIZE: u16 = 0x4;
    const MAX_FRAME_SIZE: u16 = 0x5;
    const WIDE: u32 = 1 << 30;
    let (client_io, mut server_io) = tokio::io::duplex(buffer);
    let handle = leyline::h2::start(client_io, test_config())
        .await
        .expect("handshake");
    read_preface(&mut server_io).await;
    read_frame(&mut server_io).await;
    write_server_settings_with(
        &mut server_io,
        vec![(INITIAL_WINDOW_SIZE, WIDE), (MAX_FRAME_SIZE, max_frame)],
    )
    .await;
    write_settings_ack(&mut server_io).await;
    write_window_update(&mut server_io, 0, WIDE).await;
    (handle, server_io)
}

const DEFAULT_FRAME: u32 = 16_384;
const LARGEST_FRAME: u32 = 16_777_215;

#[tokio::test]
async fn a_buffered_upload_queues_a_bounded_amount_ahead_of_its_reset() {
    const BODY: usize = 8 * 1024 * 1024;
    for max_frame in [DEFAULT_FRAME, LARGEST_FRAME] {
        let (handle, mut server_io) = wide_open(64 * 1024, max_frame).await;
        let client = handle.clone();
        let upload = tokio::spawn(async move {
            let body = RequestBody::Buffered(Bytes::from(vec![0u8; BODY]));
            client.send_shared(head("POST"), body, false).await
        });
        tokio::time::sleep(Duration::from_millis(300)).await;
        upload.abort();
        tokio::time::sleep(Duration::from_millis(500)).await;
        let queued = timeout(Duration::from_secs(5), async {
            let mut data = 0usize;
            loop {
                let (header, _) = read_frame(&mut server_io).await;
                if header.frame_type == FrameType::RstStream as u8 {
                    return data;
                }
                if header.frame_type == FrameType::Data as u8 {
                    data += header.length as usize;
                }
            }
        })
        .await
        .expect("reset reaches the peer");
        assert!(
            queued < 1024 * 1024,
            "{queued} bytes of DATA went ahead of the reset with MAX_FRAME_SIZE {max_frame}"
        );
    }
}

#[tokio::test]
async fn a_header_block_over_the_limit_is_refused_before_the_wire() {
    let (handle, mut server_io) = wide_open(4 * 1024 * 1024, DEFAULT_FRAME).await;
    let big = Head {
        pseudo: PseudoHeaders {
            method: "GET".into(),
            scheme: "https".into(),
            authority: "example.com".into(),
            path: "/".into(),
            protocol: None,
        },
        headers: vec![("x-large".into(), "v".repeat(1024 * 1024).into())],
    };
    let sent = timeout(
        Duration::from_secs(2),
        handle.send_shared(Arc::new(big), RequestBody::None, false),
    )
    .await
    .expect("the oversized request fails without waiting for a response");
    assert!(sent.is_err());
    let headers_seen = timeout(Duration::from_millis(300), async {
        loop {
            let (header, _) = read_frame(&mut server_io).await;
            if header.frame_type == FrameType::Headers as u8 {
                return;
            }
        }
    })
    .await;
    assert!(
        headers_seen.is_err(),
        "the oversized header block reached the peer"
    );
}

struct Empty(Arc<Producer>);

impl Stream for Empty {
    type Item = io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.produced.fetch_add(1, Ordering::SeqCst);
        Poll::Ready(Some(Ok(Bytes::new())))
    }
}

impl Drop for Empty {
    fn drop(&mut self) {
        self.0.dropped.store(true, Ordering::SeqCst);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_upload_of_empty_chunks_drops_its_producer() {
    let (handle, _server_io) = wide_open(1 << 20, DEFAULT_FRAME).await;
    let producer = Arc::new(Producer::default());
    let body = RequestBody::Streaming {
        stream: Box::pin(Empty(Arc::clone(&producer))),
        length_hint: None,
    };
    let upload = send(&handle, "POST", body);
    tokio::time::sleep(Duration::from_millis(100)).await;
    upload.abort();
    timeout(Duration::from_secs(2), async {
        while !producer.dropped.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("producer dropped after cancel");
}

fn head_with(headers: Vec<(String, String)>) -> Arc<Head> {
    Arc::new(Head {
        pseudo: PseudoHeaders {
            method: "GET".into(),
            scheme: "https".into(),
            authority: "example.com".into(),
            path: "/".into(),
            protocol: None,
        },
        headers: headers
            .into_iter()
            .map(|(name, value)| (name.into(), value.into()))
            .collect(),
    })
}

#[tokio::test]
async fn a_refused_header_block_leaves_the_peer_able_to_decode_the_next_request() {
    const END_HEADERS: u8 = 0x4;
    let (handle, mut server_io) = wide_open(4 * 1024 * 1024, DEFAULT_FRAME).await;
    let value = "v".repeat(1000);
    let oversized = (0..300)
        .map(|i| (format!("x-h{i}"), value.clone()))
        .collect::<Vec<_>>();
    let refused = timeout(
        Duration::from_secs(2),
        handle.send_shared(head_with(oversized), RequestBody::None, false),
    )
    .await
    .expect("the oversized request fails without waiting for a response");
    assert!(refused.is_err());

    let client = handle.clone();
    let next = head_with(vec![("x-h299".into(), value.clone())]);
    let pending =
        tokio::spawn(async move { client.send_shared(next, RequestBody::None, false).await });
    let block = timeout(Duration::from_secs(2), async {
        loop {
            let (header, payload) = read_frame(&mut server_io).await;
            if header.frame_type == FrameType::Headers as u8 {
                assert_ne!(header.flags & END_HEADERS, 0);
                return payload;
            }
        }
    })
    .await
    .expect("the next request reaches the peer");
    let decoded = leyline::h2::hpack::Decoder::new().decode_header_block(&block);
    let headers = decoded.expect("the peer decodes the next header block");
    assert!(
        headers
            .iter()
            .any(|h| &h.name[..] == b"x-h299" && h.value[..] == *value.as_bytes()),
        "{headers:?}"
    );
    pending.abort();
}

#[tokio::test]
async fn settings_acks_for_a_peer_that_stops_reading_stay_bounded() {
    use tokio::io::AsyncWriteExt;
    const SETTINGS_ACK: usize = FRAME_HEADER_LEN;
    const BOUND: usize = 512 * 1024;
    const FLOOD: usize = 200_000;
    let mut config = test_config();
    config.settings_flood_threshold = u32::MAX;
    let (client_io, mut server_io) = tokio::io::duplex(64 * 1024);
    let handle = leyline::h2::start(client_io, config)
        .await
        .expect("handshake");
    read_preface(&mut server_io).await;
    read_frame(&mut server_io).await;
    write_server_settings(&mut server_io).await;
    write_settings_ack(&mut server_io).await;
    let (mut reader, mut writer) = tokio::io::split(server_io);
    let client = handle.clone();
    let retained = tokio::spawn(async move {
        client
            .send_shared(head("GET"), RequestBody::None, true)
            .await
    });
    let mut settings = Vec::new();
    write_server_settings_with(&mut settings, vec![]).await;
    let mut sent = 0usize;
    while sent < FLOOD {
        match timeout(Duration::from_millis(500), writer.write_all(&settings)).await {
            Ok(Ok(())) => sent += 1,
            _ => break,
        }
    }
    let mut acks = 0usize;
    loop {
        let mut header = [0u8; FRAME_HEADER_LEN];
        match timeout(Duration::from_millis(500), reader.read_exact(&mut header)).await {
            Ok(Ok(_)) => {}
            _ => break,
        }
        let header = FrameHeader::parse(&header);
        let mut payload = vec![0u8; header.length as usize];
        if reader.read_exact(&mut payload).await.is_err() {
            break;
        }
        if header.frame_type == FrameType::Settings as u8 && header.flags & 0x1 == 0x1 {
            acks += 1;
        }
    }
    assert!(
        acks * SETTINGS_ACK < BOUND,
        "{acks} SETTINGS ACKs were queued for a peer that sent {sent} SETTINGS and read nothing"
    );
    assert!(!handle.is_closed());
    retained.abort();
}

#[tokio::test]
async fn the_header_list_limit_follows_the_peer_not_the_inbound_limit() {
    const MAX_HEADER_LIST_SIZE: u16 = 0x6;
    let big = || head_with(vec![("x-large".into(), "v".repeat(4096))]);

    let (client_io, mut server_io) = tokio::io::duplex(1 << 20);
    let handle = leyline::h2::start(client_io, test_config())
        .await
        .expect("handshake");
    read_preface(&mut server_io).await;
    read_frame(&mut server_io).await;
    write_server_settings_with(&mut server_io, vec![(MAX_HEADER_LIST_SIZE, 2048)]).await;
    write_settings_ack(&mut server_io).await;
    let refused = timeout(
        Duration::from_secs(2),
        handle.send_shared(big(), RequestBody::None, false),
    )
    .await
    .expect("a request over the peer's limit fails without waiting");
    assert!(refused.is_err());

    let mut config = test_config();
    config.max_header_block_bytes = 1024;
    let (client_io, mut server_io) = tokio::io::duplex(1 << 20);
    let handle = leyline::h2::start(client_io, config)
        .await
        .expect("handshake");
    read_preface(&mut server_io).await;
    read_frame(&mut server_io).await;
    write_server_settings(&mut server_io).await;
    write_settings_ack(&mut server_io).await;
    let client = handle.clone();
    let pending =
        tokio::spawn(async move { client.send_shared(big(), RequestBody::None, false).await });
    let reached = timeout(Duration::from_secs(2), async {
        loop {
            let (header, _) = read_frame(&mut server_io).await;
            if header.frame_type == FrameType::Headers as u8 {
                return;
            }
        }
    })
    .await;
    assert!(
        reached.is_ok(),
        "a small inbound limit refused an outgoing request"
    );
    pending.abort();
}
