use crate::core_support::wait;

use futures_util::TryFutureExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use leyline::testing;
use leyline::trace::{Event, Trace};
use leyline::{Browser, HostLimits, Kind, Session};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("leyline-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn slow_body() -> testing::Server {
    testing::Server::http(|_| {
        testing::Response::new(200)
            .chunks(["a", "b", "c", "d", "e", "f"], Duration::from_millis(150))
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn a_body_timeout_bounds_a_download() {
    let server = slow_body().await;
    let dir = scratch("dltimeout");
    let err = Session::new()
        .get(server.url("/file"))
        .timeout(leyline::Timeout::new().body(Duration::from_millis(300)))
        .error_for_status()
        .stream()
        .send()
        .and_then(|resp| resp.download_to(dir.join("file")))
        .await
        .unwrap_err();
    assert!(err.kind() == Kind::Timeout, "{err:?}");
    assert!(!dir.join("file").exists());
    drop(std::fs::remove_dir_all(&dir));
}

#[tokio::test]
async fn a_tab_without_a_page_sends_no_script_request() {
    let server = testing::Server::http(|_| testing::Response::new(200))
        .await
        .unwrap();
    let tab = Session::from(Browser::default()).tab();
    let err = tab
        .fetch(leyline::http::Method::GET, server.url("/api"))
        .send()
        .await
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Request);
    assert!(server.requests().await.is_empty());
}

#[tokio::test]
async fn shutdown_stops_a_body_being_streamed() {
    let server = slow_body().await;
    let session = Session::new();
    let resp = session.get(server.url("/")).stream().send().await.unwrap();
    session.shutdown().await;
    let err = resp.bytes().await.unwrap_err();
    assert!(err.is_shut_down(), "{err:?}");
}

#[tokio::test]
async fn a_streamed_body_holds_its_host_slot_until_it_ends() {
    let server = slow_body().await;
    let session = Session::builder()
        .host_limits(HostLimits::new().max_in_flight(1))
        .build()
        .unwrap();
    let held = session.get(server.url("/a")).stream().send().await.unwrap();
    let second = tokio::spawn({
        let session = session.clone();
        let url = server.url("/b");
        async move { session.get(url).send().await }
    });
    wait::until(|| {
        let stats = session.stats();
        stats.hosts().any(|h| stats.waiting(h) == 1)
    })
    .await;
    let stats = session.stats();
    let host = stats.hosts().next().unwrap();
    assert_eq!(stats.in_flight(host), 1);
    assert_eq!(stats.waiting(host), 1);
    drop(held);
    second.await.unwrap().unwrap();
}

#[derive(Default)]
struct Browsers(Mutex<Vec<Option<Browser>>>);

impl Trace for Browsers {
    fn event(&self, event: &Event<'_>) {
        if let Event::Summary { browser, .. } = event {
            self.0.lock().unwrap().push(*browser);
        }
    }
}

#[tokio::test]
async fn summaries_name_the_browser_and_pools_count_idle_connections() {
    let server = testing::Server::http(|_| testing::Response::new(200))
        .await
        .unwrap();
    let hook = Arc::new(Browsers::default());
    let session = Session::builder()
        .browser(Browser::Chrome154)
        .trace(Arc::clone(&hook))
        .build()
        .unwrap();
    session.get(server.url("/")).send().await.unwrap();
    assert_eq!(
        hook.0.lock().unwrap().as_slice(),
        [Some(Browser::Chrome154)]
    );
    let stats = session.stats();
    assert_eq!(stats.busy(), 0);
    assert!(stats.idle() >= 1, "{stats:?}");
}

#[tokio::test]
async fn a_policy_reads_the_wait_from_an_error() {
    let server = testing::Server::http(|_| {
        testing::Response::new(403)
            .header("x-ratelimit-remaining", "0")
            .header("x-wait", "9")
    })
    .await
    .unwrap();
    let policy = leyline::Retry::new(0).wait_header("x-wait");
    let err = Session::new()
        .get(server.url("/"))
        .retry(policy)
        .error_for_status()
        .send()
        .await
        .unwrap_err();
    assert_eq!(err.retry_after(), Some(Duration::from_secs(9)));
}

#[test]
fn a_changed_profile_is_named_as_such() {
    let dir = scratch("profilechanged");
    let path = dir.join("device.json");
    Session::from(Browser::Chrome154).save(&path).unwrap();
    let err = Session::load(&path)
        .unwrap()
        .browser(Browser::firefox())
        .build()
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Config);
    assert!(err.is_profile_changed(), "{err}");
    drop(std::fs::remove_dir_all(&dir));
}

#[tokio::test]
async fn a_download_refuses_a_declared_length_over_the_cap() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 2048];
        drop(socket.read(&mut buf).await);
        drop(
            socket
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 1000000\r\n\r\nabc")
                .await,
        );
        tokio::time::sleep(Duration::from_secs(10)).await;
    });
    let dir = scratch("dlcap");
    let err = tokio::time::timeout(
        Duration::from_secs(2),
        Session::new()
            .get(format!("http://{addr}/big"))
            .error_for_status()
            .max_body_size(1024)
            .stream()
            .send()
            .and_then(|resp| resp.download_to(dir.join("big"))),
    )
    .await
    .expect("refused before reading the body")
    .unwrap_err();
    assert!(err.kind() == Kind::BodyLimit, "{err:?}");
    assert!(!dir.join("big").exists());
    drop(std::fs::remove_dir_all(&dir));
}

#[tokio::test]
async fn the_session_token_stays_on_the_base_url_origin() {
    let api = testing::Server::http(|_| testing::Response::new(200))
        .await
        .unwrap();
    let other = testing::Server::http(|_| testing::Response::new(200))
        .await
        .unwrap();
    let session = Session::builder()
        .base_url(api.url("/"))
        .bearer_auth("secret-token")
        .build()
        .unwrap();
    session.get("repos").send().await.unwrap();
    session.get(other.url("/upload")).send().await.unwrap();
    session
        .get(other.url("/explicit"))
        .bearer_auth("other-token")
        .send()
        .await
        .unwrap();
    let api_seen = api.requests().await;
    assert_eq!(
        api_seen[0].header("authorization"),
        Some("Bearer secret-token")
    );
    let other_seen = other.requests().await;
    assert_eq!(other_seen[0].header("authorization"), None);
    assert_eq!(
        other_seen[1].header("authorization"),
        Some("Bearer other-token")
    );
}

#[tokio::test]
async fn a_status_error_carries_the_policy_wait_and_exhaustion() {
    let server = testing::Server::http(|_| {
        testing::Response::new(403)
            .header("x-ratelimit-remaining", "0")
            .header("x-wait", "0")
    })
    .await
    .unwrap();
    let session = Session::builder()
        .retry(
            leyline::Retry::new(1)
                .backoff(Duration::from_millis(1), Duration::from_secs(1))
                .wait_header("x-wait")
                .when(|attempt| {
                    attempt
                        .response()
                        .is_some_and(|resp| resp.header("x-ratelimit-remaining") == Some("0"))
                }),
        )
        .build()
        .unwrap();
    let err = session
        .get(server.url("/"))
        .error_for_status()
        .send()
        .await
        .unwrap_err();
    assert!(err.retries_exhausted());
    assert_eq!(err.retry_after(), Some(Duration::ZERO));
    assert_eq!(err.attempts(), 2);
}

#[tokio::test]
async fn a_429_pauses_its_host_until_retry_after() {
    let served = Arc::new(Mutex::new(0u32));
    let server = testing::Server::http({
        let served = Arc::clone(&served);
        move |_| {
            let mut n = served.lock().unwrap();
            *n += 1;
            if *n == 1 {
                testing::Response::new(429).header("retry-after", "1")
            } else {
                testing::Response::new(200)
            }
        }
    })
    .await
    .unwrap();
    let session = Session::builder()
        .host_limits(HostLimits::new().pause_on([429]))
        .build()
        .unwrap();
    assert_eq!(
        session
            .get(server.url("/a"))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16(),
        429
    );
    let started = std::time::Instant::now();
    assert_eq!(
        session
            .get(server.url("/b"))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16(),
        200
    );
    assert!(
        started.elapsed() >= Duration::from_millis(900),
        "{:?}",
        started.elapsed()
    );
}
