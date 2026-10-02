#[path = "core_support/wait.rs"]
mod wait;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use leyline::testing::{TestResponse, TestServer, queue};
use leyline::trace::{BodyEnd, BodyOutcome, Trace};
use leyline::{BlockRules, Browser, Family, HostLimits, Identity, Platform, ProxyPool, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn chrome() -> Session {
    Session::builder()
        .browser(Browser::latest(Family::Chrome))
        .build()
        .unwrap()
}

#[tokio::test]
async fn identities_share_one_pool() {
    let server = TestServer::http(|_| TestResponse::new(200)).await.unwrap();
    let base = chrome();
    let firefox = base
        .with_identity(Identity::locked(
            Browser::latest(Family::Firefox),
            Platform::Windows,
        ))
        .unwrap();
    base.get(server.url("/a")).await.unwrap();
    firefox.get(server.url("/b")).await.unwrap();
    assert_eq!(base.pool_stats().entries, 2);
    assert_eq!(firefox.pool_stats().entries, 2);
}

#[tokio::test]
async fn a_proxy_entry_carries_its_identity() {
    let proxy = TestServer::http(queue(vec![TestResponse::new(200).body("ok").close()]))
        .await
        .unwrap();
    let firefox = Identity::locked(Browser::latest(Family::Firefox), Platform::Windows);
    let session = Session::builder()
        .browser(Browser::latest(Family::Chrome))
        .proxy_pool(ProxyPool::identified([(
            format!("http://{}", proxy.addr()),
            firefox,
        )]))
        .build()
        .unwrap();
    session.get("http://origin.test/").await.unwrap();
    let sent = proxy.next_request().await.unwrap();
    let agent = sent.header_values("user-agent");
    assert!(agent[0].contains("Firefox"), "{agent:?}");
}

async fn counting_server(peak: Arc<AtomicUsize>, now: Arc<AtomicUsize>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let (peak, now) = (Arc::clone(&peak), Arc::clone(&now));
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = socket.read(&mut buf).await;
                let inside = now.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(inside, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(80)).await;
                now.fetch_sub(1, Ordering::SeqCst);
                let _ = socket
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                    .await;
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
        .host_limits(HostLimits::new().host("127.0.0.1", HostLimits::new().max_in_flight(1)))
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
    fn body(&self, ev: &BodyEnd<'_>) {
        let outcome = match ev.outcome {
            BodyOutcome::Complete => "complete",
            BodyOutcome::Dropped => "dropped",
            _ => "failed",
        };
        self.0.lock().unwrap().push((ev.bytes, outcome.to_owned()));
    }
}

#[tokio::test]
async fn streamed_bodies_report_how_they_ended() {
    let server = TestServer::http(queue(vec![
        TestResponse::new(200).close().body(vec![b'a'; 1000]),
        TestResponse::new(200).close().body(vec![b'b'; 1000]),
    ]))
    .await
    .unwrap();
    let hook = Arc::new(Ends::default());
    let session = Session::builder().trace(Arc::clone(&hook)).build().unwrap();
    let full = session.get(server.url("/full")).stream().await.unwrap();
    assert_eq!(full.bytes().await.unwrap().len(), 1000);
    let dropped = session.get(server.url("/drop")).stream().await.unwrap();
    drop(dropped);
    wait::until(|| hook.0.lock().unwrap().len() == 2).await;
    let ends = hook.0.lock().unwrap().clone();
    assert_eq!(ends[0], (1000, "complete".to_owned()));
    assert_eq!(ends[1].1, "dropped");
}

#[tokio::test]
async fn identity_sessions_keep_each_identity_cookies_apart() {
    let first = TestServer::http(queue(vec![
        TestResponse::new(403)
            .close()
            .header("set-cookie", "sid=1; Path=/"),
    ]))
    .await
    .unwrap();
    let second = TestServer::http(queue(vec![TestResponse::new(200).body("ok").close()]))
        .await
        .unwrap();
    let pool = ProxyPool::identified([
        (
            format!("http://{}", first.addr()),
            Identity::locked(Browser::latest(Family::Firefox), Platform::Windows),
        ),
        (
            format!("http://{}", second.addr()),
            Identity::locked(Browser::latest(Family::Chrome), Platform::Windows),
        ),
    ])
    .rotate_on_block(BlockRules::statuses([403]));
    let session = Session::builder()
        .browser(Browser::latest(Family::Chrome))
        .proxy_pool(pool)
        .build()
        .unwrap();
    session.get("http://origin.test/a").await.unwrap();
    session.get("http://origin.test/b").await.unwrap();
    let sent = second.next_request().await.unwrap();
    assert!(
        sent.header_values("cookie").is_empty(),
        "{:?}",
        sent.header_values("cookie")
    );
    assert!(
        session
            .cookies()
            .get_cookie(&"http://origin.test/".parse().unwrap(), "sid")
            .is_none()
    );
}

#[tokio::test]
async fn metrics_count_statuses_errors_and_attempts() {
    let server = TestServer::http(queue(vec![
        TestResponse::new(200).body("ok").close(),
        TestResponse::new(404).close(),
    ]))
    .await
    .unwrap();
    let metrics = leyline::trace::Metrics::new();
    let session = Session::builder()
        .trace(Arc::clone(&metrics))
        .build()
        .unwrap();
    session.get(server.url("/a")).await.unwrap();
    session.get(server.url("/b")).await.unwrap();
    session.get("http://127.0.0.1:9/").await.unwrap_err();
    let snap = metrics.snapshot();
    assert_eq!(snap.requests(), 3);
    assert_eq!(snap.status_class(2), 1);
    assert_eq!(snap.status_class(4), 1);
    assert_eq!(snap.errors(leyline::ErrorCategory::Connect), 1);
    assert!(snap.to_string().contains("requests=3"), "{snap}");
}

#[tokio::test]
async fn host_stats_show_requests_in_flight_and_waiting() {
    let (peak, now) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let port = counting_server(peak, now).await;
    let session = Session::builder()
        .host_limits(HostLimits::new().max_in_flight(1))
        .build()
        .unwrap();
    let url = format!("http://127.0.0.1:{port}/");
    let calls =
        futures_util::future::join_all([session.get(&url).send(), session.get(&url).send()]);
    let probe = async {
        tokio::time::sleep(Duration::from_millis(40)).await;
        session.host_stats()
    };
    let (results, stats) = tokio::join!(calls, probe);
    for result in results {
        result.unwrap();
    }
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].in_flight(), 1);
    assert_eq!(stats[0].waiting(), 1);
}
