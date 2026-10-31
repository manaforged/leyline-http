use crate::h2_support as support;
use crate::tls_support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use leyline::h2::frame::{FRAME_HEADER_LEN, FrameHeader, FrameType};
use leyline::{Browser, Protocol, Session, SessionBuilder, Tls};
use support::{read_preface, write_response, write_server_settings_with, write_settings_ack};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};

const SETTINGS_ACK: u8 = 0x1;
const MAX_CONCURRENT_STREAMS: u16 = 0x3;
const HOLD: Duration = Duration::from_millis(300);

async fn next_frame<S: AsyncRead + Unpin>(stream: &mut S) -> Option<FrameHeader> {
    let mut head = [0u8; FRAME_HEADER_LEN];
    stream.read_exact(&mut head).await.ok()?;
    let header = FrameHeader::parse(&head);
    let mut payload = vec![0u8; header.length as usize];
    stream.read_exact(&mut payload).await.ok()?;
    Some(header)
}

async fn serve<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    limit: u32,
    settings_after: Duration,
    hold: Duration,
    answers: usize,
) {
    read_preface(&mut stream).await;
    tokio::time::sleep(settings_after).await;
    write_server_settings_with(&mut stream, vec![(MAX_CONCURRENT_STREAMS, limit)]).await;
    let mut answered = 0;
    while let Some(frame) = next_frame(&mut stream).await {
        if frame.frame_type == FrameType::Settings as u8 && frame.flags & SETTINGS_ACK == 0 {
            write_settings_ack(&mut stream).await;
        }
        if frame.frame_type == FrameType::Headers as u8 {
            tokio::time::sleep(hold).await;
            write_response(&mut stream, frame.stream_id, b"ok").await;
            answered += 1;
            if answered == answers {
                return;
            }
        }
    }
}

async fn start(
    builder: SessionBuilder,
    limit: u32,
    settings_after: Duration,
    hold: Duration,
    first_answers: usize,
) -> (Session, String, Arc<AtomicUsize>) {
    let (cert, key) = tls_support::self_signed();
    let der = cert.to_der().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let port = tls_support::tls_server_per_connection(
        cert,
        key,
        Arc::clone(&connections),
        |_| tls_support::H2,
        move |index, stream| {
            let answers = if index == 0 {
                first_answers
            } else {
                usize::MAX
            };
            serve(stream, limit, settings_after, hold, answers)
        },
    )
    .await;
    let session = builder
        .protocol(Protocol::Http2)
        .tls(Tls::new().env_roots(false).system_roots(false).ca_der(der))
        .build()
        .unwrap();
    (session, format!("https://127.0.0.1:{port}/"), connections)
}

async fn send(session: &Session, url: &str, count: usize, gap: Duration) {
    let mut requests = Vec::new();
    for _ in 0..count {
        let (session, url) = (session.clone(), url.to_owned());
        requests.push(tokio::spawn(async move {
            session
                .get(&url)
                .send()
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap()
        }));
        tokio::time::sleep(gap).await;
    }
    for request in requests {
        assert_eq!(&request.await.unwrap()[..], b"ok");
    }
}

async fn connections_for(builder: SessionBuilder) -> usize {
    let (session, url, connections) = start(builder, 1, Duration::ZERO, HOLD, usize::MAX).await;
    send(&session, &url, 2, Duration::from_millis(50)).await;
    connections.load(Ordering::SeqCst)
}

#[tokio::test]
async fn a_plain_session_opens_another_connection_when_streams_are_full() {
    assert_eq!(connections_for(Session::builder()).await, 2);
}

#[tokio::test]
async fn a_browser_session_queues_on_one_connection() {
    assert_eq!(
        connections_for(Session::builder().browser(Browser::default())).await,
        1
    );
}

#[tokio::test]
async fn a_request_moves_to_a_live_connection_when_an_earlier_one_closes() {
    let (session, url, connections) = start(Session::builder(), 1, Duration::ZERO, HOLD, 1).await;
    send(&session, &url, 2, Duration::from_millis(50)).await;
    assert_eq!(connections.load(Ordering::SeqCst), 2);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let body = session
        .get(&url)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(&body[..], b"ok");
    assert_eq!(connections.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_cold_burst_spreads_before_the_peer_announces_its_limit() {
    let (session, url, connections) = start(
        Session::builder(),
        1000,
        Duration::from_millis(200),
        Duration::ZERO,
        usize::MAX,
    )
    .await;
    send(&session, &url, 150, Duration::ZERO).await;
    assert_eq!(connections.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn the_pool_counts_one_checkout_per_request() {
    let (session, url, connections) =
        start(Session::builder(), 1, Duration::ZERO, HOLD, usize::MAX).await;
    send(&session, &url, 2, Duration::from_millis(50)).await;
    send(&session, &url, 1, Duration::ZERO).await;
    assert_eq!(connections.load(Ordering::SeqCst), 2);
    let stats = session.stats();
    assert_eq!(stats.h2_hits() + stats.h2_misses(), 3);
}

#[tokio::test]
async fn cancelled_requests_give_their_stream_slots_back() {
    let (session, url, connections) = start(
        Session::builder().browser(Browser::default()),
        2,
        Duration::ZERO,
        Duration::from_millis(20),
        usize::MAX,
    )
    .await;
    let mut doomed = Vec::new();
    for _ in 0..8 {
        let (session, url) = (session.clone(), url.clone());
        doomed.push(tokio::spawn(async move { session.get(&url).send().await }));
    }
    tokio::time::sleep(Duration::from_millis(10)).await;
    for request in &doomed {
        request.abort();
    }
    tokio::time::timeout(
        Duration::from_secs(5),
        send(&session, &url, 2, Duration::ZERO),
    )
    .await
    .expect("requests after cancellation finish");
    assert_eq!(connections.load(Ordering::SeqCst), 1);
}
