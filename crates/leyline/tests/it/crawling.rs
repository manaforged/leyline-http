use crate::core_support::forward;

use futures_util::TryFutureExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use forward::counting_forwarder;
use leyline::testing::{self, queue};
use leyline::trace::{Event, Trace};
use leyline::{Body, Browser, HostLimits, Proxy, Retry, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn fast_retry() -> Retry {
    Retry::new(3).backoff(Duration::from_millis(1), Duration::from_secs(1))
}

async fn dead_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

#[tokio::test]
async fn an_http1_only_origin_takes_one_browser_handshake_and_a_streamed_body() {
    let server = testing::Server::https(queue([
        testing::Response::new(200),
        testing::Response::new(200),
    ]))
    .await
    .unwrap();
    let (port, accepted) = counting_forwarder(server.addr()).await;
    let session = Session::builder()
        .browser(Browser::default())
        .tls(server.tls())
        .build()
        .unwrap();
    let chunks = futures_util::stream::iter([
        Ok::<_, std::io::Error>(Bytes::from_static(b"part-one,")),
        Ok(Bytes::from_static(b"part-two")),
    ]);
    let upload = session
        .post(format!("https://127.0.0.1:{port}/upload"))
        .body(Body::stream(chunks, None))
        .send()
        .await
        .unwrap();
    assert_eq!(upload.status().as_u16(), 200);
    assert_eq!(accepted.load(Ordering::SeqCst), 1);

    session
        .get(format!("https://127.0.0.1:{port}/next"))
        .send()
        .await
        .unwrap();
    assert_eq!(accepted.load(Ordering::SeqCst), 1);
    let recorded = server.requests().await;
    assert_eq!(recorded[0].body, b"part-one,part-two");
}

async fn slow_server(delay: Duration) -> (u16, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let current = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (now, max) = (Arc::clone(&current), Arc::clone(&peak));
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let (now, max) = (Arc::clone(&now), Arc::clone(&max));
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                drop(socket.read(&mut buf).await);
                let inside = now.fetch_add(1, Ordering::SeqCst) + 1;
                max.fetch_max(inside, Ordering::SeqCst);
                tokio::time::sleep(delay).await;
                now.fetch_sub(1, Ordering::SeqCst);
                drop(
                    socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok",
                        )
                        .await,
                );
            });
        }
    });
    (port, current, peak)
}

#[tokio::test]
async fn host_limits_cap_requests_in_flight_per_origin() {
    let (port, _, peak) = slow_server(Duration::from_millis(100)).await;
    let session = Session::builder()
        .host_limits(HostLimits::new().max_in_flight(1))
        .build()
        .unwrap();
    let calls = (0..3).map(|_| session.get(format!("http://127.0.0.1:{port}/")).send());
    for result in futures_util::future::join_all(calls).await {
        assert_eq!(result.unwrap().status().as_u16(), 200);
    }
    assert_eq!(peak.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn host_limits_space_requests_per_second() {
    let (port, _, _) = slow_server(Duration::ZERO).await;
    let session = Session::builder()
        .host_limits(HostLimits::new().per_second(20.0))
        .build()
        .unwrap();
    let started = Instant::now();
    let calls = (0..4).map(|_| session.get(format!("http://127.0.0.1:{port}/")).send());
    for result in futures_util::future::join_all(calls).await {
        result.unwrap();
    }
    assert!(
        started.elapsed() >= Duration::from_millis(140),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_proxy_pool_bans_a_dead_proxy_and_keeps_crawling() {
    let dead = format!("http://127.0.0.1:{}", dead_port().await);
    let good = testing::Server::http(queue(vec![
        testing::Response::new(200).body("ok").close(),
        testing::Response::new(200).body("ok").close(),
        testing::Response::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();
    let pool = Proxy::list([dead.clone(), format!("http://{}", good.addr())])
        .ban_after(1)
        .ban_for(Duration::from_secs(60));
    let session = Session::builder()
        .proxy(pool.clone())
        .retry(fast_retry())
        .build()
        .unwrap();
    for path in ["/a", "/b", "/c"] {
        let resp = session
            .get(format!("http://origin.test{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
    }
    for path in ["/a", "/b", "/c"] {
        assert_eq!(
            good.next_request().await.unwrap().request_line,
            format!("GET http://origin.test{path} HTTP/1.1")
        );
    }
    let stats = session.stats();
    let banned = stats
        .proxies()
        .find(|proxy| proxy.contains(&dead[7..]))
        .unwrap();
    assert!(stats.proxy_banned_until(banned).is_some());
}

#[tokio::test]
async fn download_writes_the_decoded_body_or_nothing() {
    let dir = std::env::temp_dir().join(format!("leyline-dl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let server = testing::Server::http(queue(vec![
        testing::Response::new(200).close().body(vec![b'z'; 100]),
        testing::Response::new(404)
            .close()
            .body(b"missing".to_vec()),
    ]))
    .await
    .unwrap();
    let session = Session::builder().build().unwrap();
    let saved = dir.join("file.bin");
    let written = session
        .get(server.url("/file"))
        .error_for_status()
        .stream()
        .send()
        .and_then(|resp| resp.download_to(&saved))
        .await
        .unwrap();
    assert_eq!(written, 100);
    assert_eq!(std::fs::read(&saved).unwrap(), vec![b'z'; 100]);

    let absent = dir.join("absent.bin");
    let err = session
        .get(server.url("/absent"))
        .error_for_status()
        .stream()
        .send()
        .and_then(|resp| resp.download_to(&absent))
        .await
        .unwrap_err();
    assert_eq!(err.body(), Some(&b"missing"[..]));
    assert!(!absent.exists());
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".part"))
        .collect();
    assert!(leftovers.is_empty());
    drop(std::fs::remove_dir_all(&dir));
}

type Seen = (Option<String>, Option<String>, u32);

#[derive(Default)]
struct Labels(Mutex<Vec<Seen>>);

impl Trace for Labels {
    fn event(&self, event: &Event<'_>) {
        if let Event::Summary {
            label,
            proxy,
            attempts,
            ..
        } = event
        {
            self.0
                .lock()
                .unwrap()
                .push((label.map(str::to_owned), proxy.clone(), *attempts));
        }
    }
}

#[tokio::test]
async fn summaries_name_the_label_proxy_and_attempts() {
    let proxy = testing::Server::http(queue(vec![
        testing::Response::new(503).close(),
        testing::Response::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();
    let hook = Arc::new(Labels::default());
    let session = Session::builder()
        .proxy(format!("http://{}", proxy.addr()))
        .trace(Arc::clone(&hook))
        .retry(fast_retry())
        .build()
        .unwrap();
    let resp = session
        .get("http://origin.test/item")
        .label("job-7")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.attempts(), 2);
    let seen = hook.0.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0.as_deref(), Some("job-7"));
    assert!(
        seen[0]
            .1
            .as_deref()
            .is_some_and(|p| p.contains("127.0.0.1")),
        "{seen:?}"
    );
    assert_eq!(seen[0].2, 2);
}

#[tokio::test]
async fn a_cancelled_download_leaves_no_partial_file() {
    let dir = std::env::temp_dir().join(format!("leyline-cancel-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 2048];
        drop(socket.read(&mut buf).await);
        drop(
            socket
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 100000\r\n\r\npartial")
                .await,
        );
        tokio::time::sleep(Duration::from_secs(30)).await;
    });
    let target = dir.join("stalled.bin");
    let session = Session::builder().build().unwrap();
    let outcome = tokio::time::timeout(
        Duration::from_millis(300),
        session
            .get(format!("http://127.0.0.1:{port}/big"))
            .error_for_status()
            .stream()
            .send()
            .and_then(|resp| resp.download_to(&target)),
    )
    .await;
    outcome.expect_err("expected Err");
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name())
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    drop(std::fs::remove_dir_all(&dir));
}
