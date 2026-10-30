use crate::core_support::wait;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use leyline::testing::{self, queue};
use leyline::trace::{Event, Trace};
use leyline::{Browser, HostLimits, Platform, Proxy, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn chrome() -> Session {
    Session::builder()
        .browser(Browser::chrome())
        .build()
        .unwrap()
}

#[tokio::test]
async fn identities_share_one_pool() {
    let server = testing::Server::http(|_| testing::Response::new(200))
        .await
        .unwrap();
    let base = chrome();
    let firefox = base
        .to_builder()
        .browser(Browser::firefox())
        .platform(Platform::Windows)
        .build()
        .unwrap();
    base.get(server.url("/a")).send().await.unwrap();
    firefox.get(server.url("/b")).send().await.unwrap();
    assert_eq!(base.stats().entries(), 2);
    assert_eq!(firefox.stats().entries(), 2);
}

async fn counting_server(peak: Arc<AtomicUsize>, now: Arc<AtomicUsize>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let (peak, now) = (Arc::clone(&peak), Arc::clone(&now));
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                drop(socket.read(&mut buf).await);
                let inside = now.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(inside, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(80)).await;
                now.fetch_sub(1, Ordering::SeqCst);
                drop(
                    socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                        )
                        .await,
                );
            });
        }
    });
    port
}

#[tokio::test]
async fn host_overrides_and_a_total_cap_limit_in_flight_requests() {
    let (peak, now) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let first = counting_server(Arc::clone(&peak), Arc::clone(&now)).await;
    let second = counting_server(Arc::clone(&peak), Arc::clone(&now)).await;
    let session = Session::builder()
        .host_limits(HostLimits::new().max_total_in_flight(1))
        .build()
        .unwrap();
    let calls = (0..4).map(|i| {
        let port = if i % 2 == 0 { first } else { second };
        session.get(format!("http://127.0.0.1:{port}/")).send()
    });
    for result in futures_util::future::join_all(calls).await {
        result.unwrap();
    }
    assert_eq!(peak.load(Ordering::SeqCst), 1);

    let (peak, now) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let port = counting_server(Arc::clone(&peak), now).await;
    let session = Session::builder()
        .host_limits(HostLimits::new().host_max_in_flight("127.0.0.1", 1))
        .build()
        .unwrap();
    let calls = (0..3).map(|_| session.get(format!("http://127.0.0.1:{port}/")).send());
    for result in futures_util::future::join_all(calls).await {
        result.unwrap();
    }
    assert_eq!(peak.load(Ordering::SeqCst), 1);
}

#[derive(Default)]
struct Ends(Mutex<Vec<(u64, String)>>);

impl Trace for Ends {
    fn event(&self, event: &Event<'_>) {
        let Event::Body { bytes, outcome, .. } = event else {
            return;
        };
        let outcome = match outcome {
            Ok(()) => "complete",
            Err(None) => "dropped",
            Err(Some(_)) => "failed",
        };
        self.0.lock().unwrap().push((*bytes, outcome.to_owned()));
    }
}

#[tokio::test]
async fn streamed_bodies_report_how_they_ended() {
    let server = testing::Server::http(queue(vec![
        testing::Response::new(200).close().body(vec![b'a'; 1000]),
        testing::Response::new(200).close().body(vec![b'b'; 1000]),
    ]))
    .await
    .unwrap();
    let hook = Arc::new(Ends::default());
    let session = Session::builder().trace(Arc::clone(&hook)).build().unwrap();
    let full = session
        .get(server.url("/full"))
        .stream()
        .send()
        .await
        .unwrap();
    assert_eq!(full.bytes().await.unwrap().len(), 1000);
    let dropped = session
        .get(server.url("/drop"))
        .stream()
        .send()
        .await
        .unwrap();
    drop(dropped);
    wait::until(|| hook.0.lock().unwrap().len() == 2).await;
    let ends = hook.0.lock().unwrap().clone();
    assert_eq!(ends[0], (1000, "complete".to_owned()));
    assert_eq!(ends[1].1, "dropped");
}

#[tokio::test]
async fn identity_sessions_keep_each_identity_cookies_apart() {
    let first = testing::Server::http(queue(vec![
        testing::Response::new(403)
            .close()
            .header("set-cookie", "sid=1; Path=/"),
    ]))
    .await
    .unwrap();
    let second = testing::Server::http(queue(vec![testing::Response::new(200).body("ok").close()]))
        .await
        .unwrap();
    let pool = Proxy::identified([
        (
            format!("http://{}", first.addr()),
            Browser::firefox(),
            Platform::Windows,
        ),
        (
            format!("http://{}", second.addr()),
            Browser::chrome(),
            Platform::Windows,
        ),
    ]);
    let session = Session::builder()
        .browser(Browser::chrome())
        .proxy(pool)
        .block_statuses([403])
        .build()
        .unwrap();
    session.get("http://origin.test/a").send().await.unwrap();
    session.get("http://origin.test/b").send().await.unwrap();
    let sent = second.next_request().await.unwrap();
    assert!(
        sent.header_values("cookie").is_empty(),
        "{:?}",
        sent.header_values("cookie")
    );
    assert!(
        session
            .cookies()
            .get(&"http://origin.test/".parse().unwrap(), "sid")
            .is_none()
    );
}

#[tokio::test]
async fn metrics_count_statuses_errors_and_attempts() {
    let server = testing::Server::http(queue(vec![
        testing::Response::new(200).body("ok").close(),
        testing::Response::new(404).close(),
    ]))
    .await
    .unwrap();
    let metrics = leyline::trace::Metrics::new();
    let session = Session::builder()
        .trace(Arc::clone(&metrics))
        .build()
        .unwrap();
    session.get(server.url("/a")).send().await.unwrap();
    session.get(server.url("/b")).send().await.unwrap();
    session.get("http://127.0.0.1:9/").send().await.unwrap_err();
    let snap = &metrics;
    assert_eq!(snap.requests(), 3);
    assert_eq!(snap.status_class(2), 1);
    assert_eq!(snap.status_class(4), 1);
    assert_eq!(snap.errors(leyline::Kind::Connect), 1);
    assert!(snap.to_string().contains("requests=3"), "{snap}");
}

#[tokio::test]
async fn each_trace_call_adds_a_hook() {
    let server = testing::Server::http(queue(vec![testing::Response::new(200).body("ok").close()]))
        .await
        .unwrap();
    let first = leyline::trace::Metrics::new();
    let second = leyline::trace::Metrics::new();
    let session = Session::builder()
        .trace(Arc::clone(&first))
        .trace(Arc::clone(&second))
        .build()
        .unwrap();
    session.get(server.url("/")).send().await.unwrap();
    assert_eq!(first.requests(), 1);
    assert_eq!(second.requests(), 1);
}
