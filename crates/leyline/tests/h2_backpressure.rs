#[path = "h2_support/mod.rs"]
mod support;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use bytes::Bytes;
use futures_util::FutureExt;
use support::*;
use tokio::io::{AsyncReadExt, DuplexStream, WriteHalf};
use tokio::sync::mpsc;
use tokio::time::timeout;

use leyline::h2::frame::{FRAME_HEADER_LEN, FrameHeader, FrameType};
use leyline::h2::{H2Client, RequestBody};

const MAX_CONCURRENT_STREAMS: u16 = 0x3;
const PEER_WINDOW: usize = 65_535;
const UPLOAD_BUDGET: usize = 256 * 1024;

type Frames = mpsc::UnboundedReceiver<(FrameHeader, Vec<u8>)>;

async fn connect(settings: Vec<(u16, u32)>) -> (H2Client, WriteHalf<DuplexStream>, Frames) {
    let (client_io, mut server_io) = tokio::io::duplex(1 << 20);
    let handle = leyline::h2::start(client_io, test_config())
        .await
        .expect("handshake");
    read_preface(&mut server_io).await;
    read_frame(&mut server_io).await;
    write_server_settings_with(&mut server_io, settings).await;
    write_settings_ack(&mut server_io).await;
    let (mut reader, writer) = tokio::io::split(server_io);
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let mut header = [0u8; FRAME_HEADER_LEN];
            if reader.read_exact(&mut header).await.is_err() {
                return;
            }
            let header = FrameHeader::parse(&header);
            let mut payload = vec![0u8; header.length as usize];
            if reader.read_exact(&mut payload).await.is_err() || tx.send((header, payload)).is_err()
            {
                return;
            }
        }
    });
    (handle, writer, rx)
}

async fn next_of(frames: &mut Frames, kind: FrameType) -> (FrameHeader, Vec<u8>) {
    timeout(Duration::from_secs(3), async {
        loop {
            let (header, payload) = frames.recv().await.expect("connection open");
            if header.frame_type == kind as u8 {
                return (header, payload);
            }
        }
    })
    .await
    .expect("expected frame")
}

#[tokio::test]
async fn cancelled_queued_request_never_reaches_the_wire() {
    let (handle, mut writer, mut frames) = connect(vec![(MAX_CONCURRENT_STREAMS, 1)]).await;
    let first = send(&handle, "GET", RequestBody::None);
    let (active, _) = next_of(&mut frames, FrameType::Headers).await;
    assert_eq!(active.stream_id, 1);

    let queued = send(
        &handle,
        "POST",
        RequestBody::Buffered(Bytes::from_static(b"x")),
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    queued.abort();
    tokio::time::sleep(Duration::from_millis(50)).await;

    write_response(&mut writer, 1, b"done").await;
    timeout(Duration::from_secs(3), first)
        .await
        .expect("first request completes")
        .expect("first task");

    let probe = send(&handle, "GET", RequestBody::None);
    let (next, _) = next_of(&mut frames, FrameType::Headers).await;
    assert_eq!(next.stream_id, 3);
    assert_eq!(
        next.flags & 0x1,
        0x1,
        "stream 3 must be the body-less probe"
    );
    probe.abort();
}

#[tokio::test]
async fn stalled_upload_stops_polling_the_producer() {
    let (handle, mut writer, mut frames) = connect(vec![]).await;
    let producer = Arc::new(Producer::default());
    let upload = send(&handle, "POST", endless(&producer));

    let mut sent = 0;
    while sent < PEER_WINDOW {
        sent += next_of(&mut frames, FrameType::Data).await.1.len();
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let stalled = producer.produced.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(producer.produced.load(Ordering::SeqCst), stalled);
    assert!(
        stalled <= PEER_WINDOW + UPLOAD_BUDGET + 2 * CHUNK,
        "{stalled}"
    );

    write_window_update(&mut writer, 0, 1 << 20).await;
    write_window_update(&mut writer, 1, 1 << 20).await;
    while sent < PEER_WINDOW + UPLOAD_BUDGET {
        sent += next_of(&mut frames, FrameType::Data).await.1.len();
    }
    assert!(producer.produced.load(Ordering::SeqCst) > stalled);
    upload.abort();
}

#[tokio::test]
async fn cancelled_endless_upload_resets_the_stream_and_drops_the_producer() {
    let (handle, _writer, mut frames) = connect(vec![]).await;
    let producer = Arc::new(Producer::default());
    let upload = send(&handle, "POST", endless(&producer));
    next_of(&mut frames, FrameType::Data).await;

    upload.abort();
    let (reset, payload) = next_of(&mut frames, FrameType::RstStream).await;
    assert_eq!(reset.stream_id, 1);
    assert_eq!(payload, 0x8u32.to_be_bytes());
    timeout(Duration::from_secs(1), async {
        while !producer.dropped.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("producer dropped after cancel");
}

#[tokio::test]
async fn ping_is_sent_while_a_request_waits_for_a_slot() {
    let (handle, _writer, mut frames) = connect(vec![(MAX_CONCURRENT_STREAMS, 1)]).await;
    let first = send(&handle, "GET", RequestBody::None);
    next_of(&mut frames, FrameType::Headers).await;
    let second = send(&handle, "GET", RequestBody::None);
    tokio::time::sleep(Duration::from_millis(50)).await;

    let pinger = handle.clone();
    let ping = tokio::spawn(async move { pinger.ping().await });
    let (header, _) = next_of(&mut frames, FrameType::Ping).await;
    assert_eq!(header.flags & 0x1, 0);
    first.abort();
    second.abort();
    ping.abort();
}

#[tokio::test]
async fn request_beyond_the_command_queue_stays_with_the_caller() {
    let (handle, _writer, mut frames) = connect(vec![(MAX_CONCURRENT_STREAMS, 1)]).await;
    let mut waiting = vec![send(&handle, "GET", RequestBody::None)];
    next_of(&mut frames, FrameType::Headers).await;
    waiting.extend((0..1100).map(|_| send(&handle, "GET", RequestBody::None)));
    tokio::time::sleep(Duration::from_millis(300)).await;

    let producer = Arc::new(Producer::default());
    let late = handle.send_shared(head("POST"), endless(&producer), false);
    assert!(late.now_or_never().is_none());
    assert!(producer.dropped.load(Ordering::SeqCst));
    assert_eq!(producer.produced.load(Ordering::SeqCst), 0);
    waiting.iter().for_each(tokio::task::JoinHandle::abort);
}
