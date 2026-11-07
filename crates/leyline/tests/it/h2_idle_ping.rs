use crate::h2_support as support;
use crate::tls_support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use leyline::h2::frame::{FRAME_HEADER_LEN, FrameHeader, FrameType};
use leyline::{Session, Tls};
use leyline_bssl_tokio::SslStream;
use support::{
    read_preface, write_data, write_raw_headers, write_server_settings, write_settings_ack,
};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::TcpStream;

const SETTINGS_ACK: u8 = 0x1;
const IDLE: Duration = Duration::from_millis(50);
const PING_TIMEOUT: Duration = Duration::from_millis(300);

#[derive(Clone, Copy)]
enum First {
    AnswersButNeverAcksPings,
    GoesSilentAfterOneAnswer,
}

type Log = Arc<Mutex<Vec<u8>>>;

async fn next_frame<S: AsyncRead + Unpin>(stream: &mut S) -> Option<FrameHeader> {
    let mut head = [0u8; FRAME_HEADER_LEN];
    stream.read_exact(&mut head).await.ok()?;
    let header = FrameHeader::parse(&head);
    let mut payload = vec![0u8; header.length as usize];
    stream.read_exact(&mut payload).await.ok()?;
    Some(header)
}

async fn serve(mut stream: SslStream<TcpStream>, body: &'static [u8], first: Option<(First, Log)>) {
    read_preface(&mut stream).await;
    write_server_settings(&mut stream).await;
    let mut answered = 0;
    while let Some(frame) = next_frame(&mut stream).await {
        if let Some((_, log)) = &first {
            log.lock().unwrap().push(frame.frame_type);
        }
        if frame.frame_type == FrameType::Settings as u8 && frame.flags & SETTINGS_ACK == 0 {
            write_settings_ack(&mut stream).await;
        }
        if frame.frame_type != FrameType::Headers as u8 {
            continue;
        }
        answered += 1;
        if answered > 1 && matches!(first, Some((First::GoesSilentAfterOneAnswer, _))) {
            continue;
        }
        write_raw_headers(&mut stream, frame.stream_id, &[(":status", "200")], false).await;
        write_data(&mut stream, frame.stream_id, body, true).await;
    }
}

struct Origin {
    url: String,
    session: Session,
    connections: Arc<AtomicUsize>,
    log: Log,
}

async fn origin(first: First) -> Origin {
    let (cert, key) = tls_support::self_signed();
    let der = cert.to_der().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let log = Log::default();
    let seen = Arc::clone(&log);
    let port = tls_support::tls_server_per_connection(
        cert,
        key,
        Arc::clone(&connections),
        |_| tls_support::H2,
        move |index, stream| {
            let seen = Arc::clone(&seen);
            async move {
                if index == 0 {
                    serve(stream, b"first", Some((first, seen))).await;
                } else {
                    serve(stream, b"fresh", None).await;
                }
            }
        },
    )
    .await;
    let session = Session::builder()
        .tls(Tls::new().env_roots(false).system_roots(false).ca_der(der))
        .http2_ping_after_idle(IDLE)
        .http2_ping_timeout(PING_TIMEOUT)
        .build()
        .unwrap();
    let url = format!("https://127.0.0.1:{port}/");
    let warm = session.get(&url).send().await.unwrap();
    assert_eq!(warm.text().await.unwrap(), "first");
    tokio::time::sleep(IDLE * 3).await;
    Origin {
        url,
        session,
        connections,
        log,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_idle_get_goes_out_behind_its_ping_without_waiting_for_the_ack() {
    let origin = origin(First::AnswersButNeverAcksPings).await;
    let started = Instant::now();
    let reused = origin.session.get(&origin.url).send().await.unwrap();
    assert_eq!(reused.text().await.unwrap(), "first");
    assert!(started.elapsed() < PING_TIMEOUT, "{:?}", started.elapsed());
    assert_eq!(origin.connections.load(Ordering::SeqCst), 1);
    let frames = origin.log.lock().unwrap().clone();
    let ping = frames.iter().position(|&t| t == FrameType::Ping as u8);
    let second = frames
        .iter()
        .enumerate()
        .filter(|&(_, &t)| t == FrameType::Headers as u8)
        .nth(1)
        .map(|(at, _)| at);
    assert!(ping.is_some() && ping < second, "{frames:?}");

    tokio::time::sleep(PING_TIMEOUT * 2).await;
    let after = origin.session.get(&origin.url).send().await.unwrap();
    assert_eq!(after.text().await.unwrap(), "fresh");
    assert_eq!(origin.connections.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_idle_post_waits_for_the_ping_and_moves_to_a_fresh_connection() {
    let origin = origin(First::AnswersButNeverAcksPings).await;
    let posted = origin
        .session
        .post(&origin.url)
        .body("once")
        .send()
        .await
        .unwrap();
    assert_eq!(posted.text().await.unwrap(), "fresh");
    assert_eq!(origin.connections.load(Ordering::SeqCst), 2);
    let headers = origin
        .log
        .lock()
        .unwrap()
        .iter()
        .filter(|&&t| t == FrameType::Headers as u8)
        .count();
    assert_eq!(
        headers, 1,
        "the POST must not reach the unconfirmed connection"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_idle_get_on_a_dead_connection_is_resent_on_a_fresh_one() {
    let origin = origin(First::GoesSilentAfterOneAnswer).await;
    let resent = tokio::time::timeout(PING_TIMEOUT * 4, origin.session.get(&origin.url).send())
        .await
        .expect("the PING deadline must fail the dead connection")
        .unwrap();
    assert_eq!(resent.text().await.unwrap(), "fresh");
    assert_eq!(origin.connections.load(Ordering::SeqCst), 2);
}
