use crate::core_support::wait;

use std::time::{Duration, Instant};

use leyline::testing::{TestResponse, TestServer};
use leyline::{HostLimits, ProxyPool, RetryPolicy, Session};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("leyline-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn a_duration_timeout_does_not_bound_a_streamed_body() {
    let server = TestServer::http(|_| {
        TestResponse::new(200).chunks(["a", "b", "c", "d"], Duration::from_millis(150))
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
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(&body[..], b"abcd");
}

#[tokio::test]
async fn a_huge_retry_after_pauses_without_panicking() {
    let server =
        TestServer::http(|_| TestResponse::new(429).header("retry-after", "18446744073709551615"))
            .await
            .unwrap();
    let session = Session::builder()
        .host_limits(HostLimits::new().pause_on([429]))
        .build()
        .unwrap();
    let resp = session.get(server.url("/")).await.unwrap();
    assert_eq!(resp.status().as_u16(), 429);
}

#[tokio::test]
async fn pages_drop_credentials_when_the_next_page_is_on_another_origin() {
    let other = TestServer::http(|_| TestResponse::new(200).body("two"))
        .await
        .unwrap();
    let next = other.url("/page2");
    let first = TestServer::http(move |_| {
        TestResponse::new(200)
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
    let server = TestServer::http(|_| TestResponse::new(503).header("retry-after", "60"))
        .await
        .unwrap();
    let session = Session::builder()
        .retry(RetryPolicy::transient())
        .build()
        .unwrap();
    let pending = tokio::spawn({
        let session = session.clone();
        let url = server.url("/");
        async move { session.get(url).send().await }
    });
    server.next_request().await.unwrap();
    session.shutdown();
    let err = tokio::time::timeout(Duration::from_secs(2), pending)
        .await
        .expect("shutdown ends the backoff")
        .unwrap()
        .unwrap_err();
    assert!(err.is_shut_down(), "{err:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paused_host_does_not_hold_the_total_cap() {
    let paused = TestServer::http(|req| {
        if req.target == "/first" {
            TestResponse::new(429).header("retry-after", "5")
        } else {
            TestResponse::new(200)
        }
    })
    .await
    .unwrap();
    let other = TestServer::http(|_| TestResponse::new(200)).await.unwrap();
    let session = Session::builder()
        .host_limits(HostLimits::new().max_total_in_flight(1).pause_on([429]))
        .build()
        .unwrap();
    session.get(paused.url("/first")).await.unwrap();
    let waiting = tokio::spawn({
        let session = session.clone();
        let url = paused.url("/second");
        async move { session.get(url).await }
    });
    let port = format!(":{}", paused.addr().port());
    wait::until(|| {
        session
            .host_stats()
            .iter()
            .any(|s| s.origin().ends_with(&port) && s.in_flight() + s.waiting() > 0)
    })
    .await;
    let started = Instant::now();
    session.get(other.url("/")).await.unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    waiting.abort();
}

#[tokio::test]
async fn a_status_error_body_that_never_ends_does_not_hang_the_request() {
    let server = TestServer::http(|_| {
        TestResponse::new(401).chunks(["denied", "never"], Duration::from_secs(30))
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
    let server = TestServer::http(|_| TestResponse::new(503)).await.unwrap();
    let err = Session::builder()
        .retry(RetryPolicy::transient().initial_backoff(Duration::from_millis(1)))
        .build()
        .unwrap()
        .post(server.url("/"))
        .body("x")
        .error_for_status()
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
        .websocket("ws://127.0.0.1:9/feed")
        .connect()
        .await
        .err()
        .unwrap();
    assert_eq!(err.kind(), leyline::Kind::Request);
}

#[cfg(unix)]
#[test]
fn saved_jars_are_always_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("jarmode");
    let jar = leyline::cookie::Jar::new();
    let fresh = dir.join("fresh.json");
    jar.save_to(&fresh).unwrap();
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&fresh), 0o600);
    std::fs::set_permissions(&fresh, std::fs::Permissions::from_mode(0o640)).unwrap();
    jar.save_to(&fresh).unwrap();
    assert_eq!(mode(&fresh), 0o600);
    drop(std::fs::remove_dir_all(&dir));
}

#[tokio::test]
async fn a_ban_of_any_length_still_bans() {
    let pool = ProxyPool::new(["http://127.0.0.1:9"])
        .ban_after(1)
        .ban_for(Duration::MAX);
    let session = Session::builder().proxy_pool(pool.clone()).build().unwrap();
    drop(session.get("http://origin.test/").await);
    let health = pool.stats();
    assert!(health[0].banned_until.is_some(), "{health:?}");
}

#[tokio::test]
async fn the_first_autosave_flush_writes_the_file() {
    let dir = scratch("firstflush");
    let path = dir.join("jar.json");
    let autosave = leyline::cookie::Jar::new().autosave(&path, Duration::from_secs(60));
    autosave.flush().await.unwrap();
    assert!(path.exists());
    autosave.shutdown().await.unwrap();
    drop(std::fs::remove_dir_all(&dir));
}

#[tokio::test]
async fn the_first_https_request_reports_a_fresh_connection() {
    let server = TestServer::https(|_| TestResponse::new(200)).await.unwrap();
    let session = Session::builder()
        .tls_trust(server.trust())
        .build()
        .unwrap();
    let first = session.get(server.url("/a")).await.unwrap();
    assert!(!first.timing().reused, "{:?}", first.timing());
    assert!(first.timing().connect_ms.is_some(), "{:?}", first.timing());
    let misses = session.pool_stats().h2_misses;
    let second = session.get(server.url("/b")).await.unwrap();
    assert!(second.timing().reused, "{:?}", second.timing());
    session.get(server.url("/c")).await.unwrap();
    let stats = session.pool_stats();
    assert_eq!(stats.h2_misses, misses, "{stats:?}");
    assert_eq!(stats.evictions_dead, 0, "{stats:?}");
}

#[tokio::test]
async fn a_body_stream_ends_after_its_deadline() {
    let server = TestServer::http(|_| {
        TestResponse::new(200).chunks(["a", "b", "c"], Duration::from_millis(400))
    })
    .await
    .unwrap();
    let session = Session::builder()
        .host_limits(HostLimits::new().max_in_flight(1))
        .build()
        .unwrap();
    let mut body = session
        .get(server.url("/"))
        .timeout(leyline::TimeoutConfig::new().body(Duration::from_millis(200)))
        .stream()
        .await
        .unwrap()
        .into_stream()
        .unwrap();
    let mut errors = 0;
    while let Some(chunk) = futures_util::StreamExt::next(&mut body).await {
        if chunk.is_err() {
            errors += 1;
        }
        assert!(errors <= 1, "the stream kept failing");
    }
    assert_eq!(errors, 1);
    assert!(session.host_stats().iter().all(|s| s.in_flight() == 0));
}

#[tokio::test]
async fn a_streamed_body_ends_at_its_first_error() {
    let server =
        TestServer::http(|_| TestResponse::new(200).chunks(["a", "b"], Duration::from_millis(500)))
            .await
            .unwrap();
    let mut body = Session::new()
        .get(server.url("/"))
        .timeout(leyline::TimeoutConfig::new().read(Duration::from_millis(200)))
        .stream()
        .await
        .unwrap()
        .into_stream()
        .unwrap();
    use futures_util::StreamExt;
    assert_eq!(&body.next().await.unwrap().unwrap()[..], b"a");
    body.next().await.unwrap().expect_err("expected Err");
    assert!(body.next().await.is_none());
}
