use crate::core_support::wait;
use futures_util::StreamExt;

use std::time::{Duration, Instant};

use leyline::testing;
use leyline::{HostLimits, Proxy, Retry, Session};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("leyline-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn a_duration_timeout_does_not_bound_a_streamed_body() {
    let server = testing::Server::http(|_| {
        testing::Response::new(200).chunks(["a", "b", "c", "d"], Duration::from_millis(150))
    })
    .await
    .unwrap();
    let session = Session::builder()
        .timeout(Duration::from_millis(300))
        .build()
        .unwrap();
    let body = session
        .get(server.url("/feed"))
        .stream()
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(&body[..], b"abcd");
}

#[tokio::test]
async fn a_huge_retry_after_pauses_without_panicking() {
    let server = testing::Server::http(|_| {
        testing::Response::new(429).header("retry-after", "18446744073709551615")
    })
    .await
    .unwrap();
    let session = Session::builder()
        .host_limits(HostLimits::new().pause_on([429]))
        .build()
        .unwrap();
    let resp = session.get(server.url("/")).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 429);
}

#[tokio::test]
async fn pages_drop_credentials_when_the_next_page_is_on_another_origin() {
    let other = testing::Server::http(|_| testing::Response::new(200).body("two"))
        .await
        .unwrap();
    let next = other.url("/page2");
    let first = testing::Server::http(move |_| {
        testing::Response::new(200)
            .header("link", format!("<{next}>; rel=\"next\""))
            .body("one")
    })
    .await
    .unwrap();
    let mut pages = Session::new()
        .get(first.url("/page1"))
        .bearer_auth("secret")
        .pages();
    while let Some(page) = pages.next().await {
        page.unwrap();
    }
    assert_eq!(
        first.requests().await[0].header("authorization"),
        Some("Bearer secret")
    );
    let seen = other.requests().await;
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].header("authorization"), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_interrupts_a_retry_backoff() {
    let server = testing::Server::http(|_| testing::Response::new(503).header("retry-after", "60"))
        .await
        .unwrap();
    let session = Session::builder().retry(3).build().unwrap();
    let pending = tokio::spawn({
        let session = session.clone();
        let url = server.url("/");
        async move { session.get(url).send().await }
    });
    server.next_request().await.unwrap();
    session.shutdown().await;
    let err = tokio::time::timeout(Duration::from_secs(2), pending)
        .await
        .expect("shutdown ends the backoff")
        .unwrap()
        .unwrap_err();
    assert!(err.is_shut_down(), "{err:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paused_host_does_not_hold_the_total_cap() {
    let paused = testing::Server::http(|req| {
        if req.target == "/first" {
            testing::Response::new(429).header("retry-after", "5")
        } else {
            testing::Response::new(200)
        }
    })
    .await
    .unwrap();
    let other = testing::Server::http(|_| testing::Response::new(200))
        .await
        .unwrap();
    let session = Session::builder()
        .host_limits(HostLimits::new().max_total_in_flight(1).pause_on([429]))
        .build()
        .unwrap();
    session.get(paused.url("/first")).send().await.unwrap();
    let waiting = tokio::spawn({
        let session = session.clone();
        let url = paused.url("/second");
        async move { session.get(url).send().await }
    });
    let port = format!(":{}", paused.addr().port());
    wait::until(|| {
        let stats = session.stats();
        stats
            .hosts()
            .any(|h| h.ends_with(&port) && stats.in_flight(h) + stats.waiting(h) > 0)
    })
    .await;
    let started = Instant::now();
    session.get(other.url("/")).send().await.unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    waiting.abort();
}

#[tokio::test]
async fn a_status_error_body_that_never_ends_does_not_hang_the_request() {
    let server = testing::Server::http(|_| {
        testing::Response::new(401).chunks(["denied", "never"], Duration::from_secs(30))
    })
    .await
    .unwrap();
    let started = Instant::now();
    let err = Session::new()
        .get(server.url("/"))
        .timeout(Duration::from_millis(500))
        .stream()
        .error_for_status()
        .send()
        .await
        .unwrap_err();
    assert_eq!(err.status().map(|s| s.as_u16()), Some(401));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_post_that_was_never_retried_is_not_exhausted() {
    let server = testing::Server::http(|_| testing::Response::new(503))
        .await
        .unwrap();
    let err = Session::builder()
        .retry(Retry::new(3).backoff(Duration::from_millis(1), Duration::from_secs(1)))
        .build()
        .unwrap()
        .post(server.url("/"))
        .body("x")
        .error_for_status()
        .send()
        .await
        .unwrap_err();
    assert_eq!(err.attempts(), 1);
    assert!(!err.retries_exhausted());
}

#[tokio::test]
async fn a_plaintext_websocket_is_refused_before_sending() {
    let session = Session::builder()
        .base_url("https://127.0.0.1:9/")
        .bearer_auth("secret")
        .build()
        .unwrap();
    let err = session
        .get("ws://127.0.0.1:9/feed")
        .websocket()
        .await
        .err()
        .unwrap();
    assert_eq!(err.kind(), leyline::Kind::Request);
}

#[cfg(unix)]
#[test]
fn saved_devices_are_always_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("jarmode");
    let session = Session::new();
    let fresh = dir.join("fresh.json");
    session.save(&fresh).unwrap();
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&fresh), 0o600);
    std::fs::set_permissions(&fresh, std::fs::Permissions::from_mode(0o640)).unwrap();
    session.save(&fresh).unwrap();
    assert_eq!(mode(&fresh), 0o600);
    drop(std::fs::remove_dir_all(&dir));
}

#[tokio::test]
async fn a_ban_of_any_length_still_bans() {
    let pool = Proxy::list(["http://127.0.0.1:9"])
        .ban_after(1)
        .ban_for(Duration::MAX);
    let session = Session::builder().proxy(pool.clone()).build().unwrap();
    drop(session.get("http://origin.test/").send().await);
    let stats = session.stats();
    let proxy = stats.proxies().next().unwrap();
    assert!(stats.proxy_banned_until(proxy).is_some(), "{stats:?}");
}

#[tokio::test]
async fn the_first_https_request_reports_a_fresh_connection() {
    let server = testing::Server::https(|_| testing::Response::new(200))
        .await
        .unwrap();
    let session = Session::builder().tls(server.tls()).build().unwrap();
    let first = session.get(server.url("/a")).send().await.unwrap();
    assert!(!first.reused());
    assert!(first.connect_time().is_some());
    let misses = session.stats().h2_misses();
    let second = session.get(server.url("/b")).send().await.unwrap();
    assert!(second.reused());
    session.get(server.url("/c")).send().await.unwrap();
    let stats = session.stats();
    assert_eq!(stats.h2_misses(), misses, "{stats:?}");
    assert_eq!(stats.evictions_dead(), 0, "{stats:?}");
}

#[tokio::test]
async fn a_body_stream_ends_after_its_deadline() {
    let server = testing::Server::http(|_| {
        testing::Response::new(200).chunks(["a", "b", "c"], Duration::from_millis(400))
    })
    .await
    .unwrap();
    let session = Session::builder()
        .host_limits(HostLimits::new().max_in_flight(1))
        .build()
        .unwrap();
    let mut body = session
        .get(server.url("/"))
        .timeout(leyline::Timeout::new().body(Duration::from_millis(200)))
        .stream()
        .send()
        .await
        .unwrap()
        .bytes_stream()
        .unwrap();
    let mut errors = 0;
    while let Some(chunk) = body.next().await {
        if chunk.is_err() {
            errors += 1;
        }
        assert!(errors <= 1, "the stream kept failing");
    }
    assert_eq!(errors, 1);
    assert!({
        let stats = session.stats();
        stats.hosts().all(|h| stats.in_flight(h) == 0)
    });
}

#[tokio::test]
async fn a_streamed_body_ends_at_its_first_error() {
    let server = testing::Server::http(|_| {
        testing::Response::new(200).chunks(["a", "b"], Duration::from_millis(500))
    })
    .await
    .unwrap();
    let mut body = Session::new()
        .get(server.url("/"))
        .timeout(leyline::Timeout::new().read(Duration::from_millis(200)))
        .stream()
        .send()
        .await
        .unwrap()
        .bytes_stream()
        .unwrap();
    assert_eq!(&body.next().await.unwrap().unwrap()[..], b"a");
    body.next().await.unwrap().expect_err("expected Err");
    assert!(body.next().await.is_none());
}
