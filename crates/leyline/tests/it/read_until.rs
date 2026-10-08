use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use leyline::testing::{TestResponse, TestServer, queue};
use leyline::{HostLimits, Kind, ProtocolPolicy, Session, StopReason};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const MARKER: &[u8] = b"in-stock";

fn has_marker(body: &[u8], from: usize) -> bool {
    let start = from.saturating_sub(MARKER.len() - 1);
    body[start..].windows(MARKER.len()).any(|w| w == MARKER)
}

fn never(_: &[u8], _: usize) -> bool {
    false
}

fn http1() -> Session {
    Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap()
}

async fn read_head(socket: &mut TcpStream) -> bool {
    let mut seen = Vec::new();
    let mut buf = [0u8; 1024];
    while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
        match socket.read(&mut buf).await {
            Ok(0) | Err(_) => return false,
            Ok(n) => seen.extend_from_slice(&buf[..n]),
        }
    }
    true
}

async fn product_page_server(closed_early: Arc<AtomicBool>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let closed_early = Arc::clone(&closed_early);
            tokio::spawn(async move {
                if !read_head(&mut socket).await {
                    return;
                }
                let total = 4 * 1024 * 1024;
                let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {total}\r\n\r\n");
                drop(socket.write_all(head.as_bytes()).await);
                drop(socket.write_all(b"<div class=\"stock\">in-st").await);
                tokio::time::sleep(Duration::from_millis(100)).await;
                drop(socket.write_all(b"ock</div>").await);
                let mut probe = [0u8; 1];
                if let Ok(Ok(0)) =
                    tokio::time::timeout(Duration::from_secs(2), socket.read(&mut probe)).await
                {
                    closed_early.store(true, Ordering::SeqCst);
                    return;
                }
                drop(socket.write_all(&vec![b'z'; total]).await);
            });
        }
    });
    port
}

#[tokio::test]
async fn a_marker_split_across_chunks_stops_the_transfer_early() {
    let closed_early = Arc::new(AtomicBool::new(false));
    let port = product_page_server(Arc::clone(&closed_early)).await;
    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .host_limits(HostLimits::new().max_in_flight(1))
        .build()
        .unwrap();
    let url = format!("http://127.0.0.1:{port}/item");

    let first = session
        .get(&url)
        .read_until(64 * 1024, has_marker)
        .await
        .unwrap();

    assert_eq!(first.stopped_by, StopReason::PredicateMatched);
    assert!(first.bytes.ends_with(b"ock</div>"), "{:?}", first.bytes);
    let started = Instant::now();
    let second = session
        .get(&url)
        .read_until(64 * 1024, has_marker)
        .await
        .unwrap();
    assert_eq!(second.stopped_by, StopReason::PredicateMatched);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    assert!(closed_early.load(Ordering::SeqCst));
}

#[tokio::test]
async fn a_marker_split_across_gzip_chunks_matches() {
    let mut page = vec![b'x'; 5000];
    page.extend_from_slice(MARKER);
    page.extend_from_slice(&[b'y'; 5000]);
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&page).unwrap();
    let gz = encoder.finish().unwrap();
    let parts: Vec<Vec<u8>> = gz.chunks(16).map(<[u8]>::to_vec).collect();
    let server = TestServer::http(queue([TestResponse::new(200)
        .header("content-encoding", "gzip")
        .chunks(parts, Duration::ZERO)]))
    .await
    .unwrap();

    let read = http1()
        .get(server.url("/"))
        .read_until(1 << 20, has_marker)
        .await
        .unwrap();

    assert_eq!(read.stopped_by, StopReason::PredicateMatched);
    assert!(read.bytes.len() >= 5000 + MARKER.len());
    assert!(read.bytes.len() < page.len(), "{}", read.bytes.len());
}

#[tokio::test]
async fn the_limit_is_reached_without_a_match() {
    let server = TestServer::http(queue([TestResponse::new(200).body(vec![b'a'; 1000])]))
        .await
        .unwrap();

    let read = http1()
        .get(server.url("/"))
        .read_until(100, never)
        .await
        .unwrap();

    assert_eq!(read.stopped_by, StopReason::LimitReached);
    assert_eq!(read.bytes, vec![b'a'; 100]);
}

#[tokio::test]
async fn a_match_inside_the_limit_wins_and_one_past_it_does_not() {
    let body = "aaaaMARKERbbbb";
    let marker = |b: &[u8], _: usize| b.windows(6).any(|w| w == b"MARKER");
    let server = TestServer::http(queue([
        TestResponse::new(200).body(body),
        TestResponse::new(200).body(body),
    ]))
    .await
    .unwrap();
    let session = http1();

    let cut = session
        .get(server.url("/"))
        .read_until(9, marker)
        .await
        .unwrap();
    let whole = session
        .get(server.url("/"))
        .read_until(10, marker)
        .await
        .unwrap();

    assert_eq!(
        (cut.stopped_by, cut.bytes),
        (StopReason::LimitReached, b"aaaaMARKE".to_vec())
    );
    assert_eq!(whole.stopped_by, StopReason::PredicateMatched);
    assert_eq!(whole.bytes, b"aaaaMARKER");
}

#[tokio::test]
async fn the_end_of_the_body_without_a_match_is_reported() {
    let server = TestServer::http(queue([
        TestResponse::new(200).chunks(["ab", "cd", "ef"], Duration::from_millis(20))
    ]))
    .await
    .unwrap();
    let mut calls = Vec::new();

    let read = http1()
        .get(server.url("/"))
        .read_until(100, |body, from| {
            calls.push((body.len(), from));
            false
        })
        .await
        .unwrap();

    assert_eq!(read.stopped_by, StopReason::EndOfBody);
    assert_eq!(read.bytes, b"abcdef");
    assert_eq!(calls, [(2, 0), (4, 2), (6, 4)]);
}

#[tokio::test]
async fn an_empty_body_and_a_zero_limit_never_call_the_predicate() {
    let server = TestServer::http(queue([
        TestResponse::new(200),
        TestResponse::new(200).body("content"),
    ]))
    .await
    .unwrap();
    let session = http1();
    let mut called = false;

    let empty = session
        .get(server.url("/"))
        .read_until(10, |_, _| {
            called = true;
            true
        })
        .await
        .unwrap();
    let zero = session
        .get(server.url("/"))
        .read_until(0, never)
        .await
        .unwrap();

    assert!(!called);
    assert_eq!(
        (empty.stopped_by, empty.bytes),
        (StopReason::EndOfBody, Vec::new())
    );
    assert_eq!(
        (zero.stopped_by, zero.bytes),
        (StopReason::LimitReached, Vec::new())
    );
}

#[tokio::test]
async fn error_for_status_still_applies() {
    let server = TestServer::http(queue([TestResponse::new(503).body("busy")]))
        .await
        .unwrap();

    let err = http1()
        .get(server.url("/"))
        .error_for_status()
        .read_until(100, never)
        .await
        .unwrap_err();

    assert_eq!(err.status().map(|s| s.as_u16()), Some(503));
    assert_eq!(err.body(), Some(&b"busy"[..]));
}

#[tokio::test]
async fn a_body_cut_short_is_an_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_head(&mut socket).await;
        drop(
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial")
                .await,
        );
    });

    let err = http1()
        .get(format!("http://127.0.0.1:{port}/"))
        .read_until(1000, never)
        .await
        .unwrap_err();

    assert_eq!(err.kind(), Kind::Io, "{err:?}");
}

#[tokio::test]
async fn a_body_that_fails_to_decode_is_a_decode_error() {
    let server = TestServer::http(queue([TestResponse::new(200)
        .header("content-encoding", "gzip")
        .body("this is not gzip")]))
    .await
    .unwrap();

    let err = http1()
        .get(server.url("/"))
        .read_until(1000, never)
        .await
        .unwrap_err();

    assert_eq!(err.kind(), Kind::Decode, "{err:?}");
}
