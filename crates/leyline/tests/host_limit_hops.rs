use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use leyline::testing::{TestResponse, TestServer};
use leyline::{HostLimits, RetryPolicy, Session, WaitFormat};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn slow_origin(delay: Duration) -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let now = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (n, p) = (Arc::clone(&now), Arc::clone(&peak));
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let (n, p) = (Arc::clone(&n), Arc::clone(&p));
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = socket.read(&mut buf).await;
                let inside = n.fetch_add(1, Ordering::SeqCst) + 1;
                p.fetch_max(inside, Ordering::SeqCst);
                tokio::time::sleep(delay).await;
                n.fetch_sub(1, Ordering::SeqCst);
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok",
                    )
                    .await;
            });
        }
    });
    (port, peak)
}

async fn redirect_to(target: String) -> TestServer {
    TestServer::http(move |_| {
        TestResponse::new(302)
            .close()
            .header("location", target.clone())
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn a_redirect_hop_waits_for_the_target_host_slot() {
    let (port, peak) = slow_origin(Duration::from_millis(150)).await;
    let target = format!("http://127.0.0.1:{port}/");
    let mut redirectors = Vec::new();
    for _ in 0..4 {
        redirectors.push(redirect_to(target.clone()).await);
    }
    let session = Session::builder()
        .host_limits(HostLimits::new().max_in_flight(1))
        .build()
        .unwrap();
    let calls = redirectors.iter().map(|a| session.get(a.url("/")).send());
    for result in futures_util::future::join_all(calls).await {
        assert_eq!(result.unwrap().status().as_u16(), 200);
    }
    assert_eq!(peak.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_pause_lands_on_the_host_that_answered() {
    let answering = TestServer::http(|_| TestResponse::new(429).close().header("retry-after", "5"))
        .await
        .unwrap();
    let first = redirect_to(answering.url("/")).await;
    let session = Session::builder()
        .host_limits(HostLimits::new().pause_on([429]))
        .build()
        .unwrap();
    assert_eq!(
        session.get(first.url("/")).await.unwrap().status().as_u16(),
        429
    );
    let direct = tokio::time::timeout(
        Duration::from_millis(500),
        session.get(answering.url("/again")).send(),
    )
    .await;
    assert!(direct.is_err(), "the answering host was not paused");
}

#[tokio::test]
async fn a_trailing_dot_names_the_same_host() {
    let (port, peak) = slow_origin(Duration::from_millis(150)).await;
    let session = Session::builder()
        .host_limits(HostLimits::new().max_in_flight(1))
        .build()
        .unwrap();
    let (a, b) = tokio::join!(
        session.get(format!("http://localhost:{port}/")).send(),
        session.get(format!("http://localhost.:{port}/")).send(),
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(peak.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn each_session_built_from_one_limit_has_its_own_slots() {
    let (port, peak) = slow_origin(Duration::from_millis(200)).await;
    let limits = HostLimits::new().max_in_flight(1);
    let one = Session::builder()
        .host_limits(limits.clone())
        .build()
        .unwrap();
    let two = Session::builder().host_limits(limits).build().unwrap();
    let url = format!("http://127.0.0.1:{port}/");
    let (a, b) = tokio::join!(one.get(&url).send(), two.get(&url).send());
    a.unwrap();
    b.unwrap();
    assert_eq!(peak.load(Ordering::SeqCst), 2);
}

#[test]
fn an_invalid_pause_status_fails_the_build() {
    let err = Session::builder()
        .host_limits(HostLimits::new().pause_on([42]))
        .build()
        .unwrap_err();
    assert_eq!(err.kind(), leyline::Kind::Config);
}

#[tokio::test]
async fn the_pause_follows_the_policy_wait_header() {
    let server = TestServer::http(leyline::testing::queue([
        TestResponse::new(429).close().header("x-wait", "1"),
        TestResponse::new(200).close(),
    ]))
    .await
    .unwrap();
    let session = Session::builder()
        .host_limits(
            HostLimits::new()
                .pause_on([429])
                .pause_for(Duration::from_secs(60)),
        )
        .retry(RetryPolicy::none().wait_header("x-wait", WaitFormat::Seconds))
        .build()
        .unwrap();
    session.get(server.url("/")).await.unwrap();
    let started = Instant::now();
    let next = tokio::time::timeout(Duration::from_secs(5), session.get(server.url("/")).send())
        .await
        .expect("paused for the 60 s default instead of the 1 s header")
        .unwrap();
    assert_eq!(next.status().as_u16(), 200);
    assert!(started.elapsed() >= Duration::from_millis(900));
}

#[tokio::test]
async fn rate_spacing_holds_when_the_total_cap_is_busy() {
    let (slow, _) = slow_origin(Duration::from_millis(800)).await;
    let arrivals = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = Arc::clone(&arrivals);
    let spaced = TestServer::http(move |_| {
        seen.lock().unwrap().push(Instant::now());
        TestResponse::new(200).close()
    })
    .await
    .unwrap();
    let session = Session::builder()
        .host_limits(
            HostLimits::new()
                .max_total_in_flight(1)
                .host("localhost", HostLimits::new().per_second(4.0)),
        )
        .build()
        .unwrap();
    let blocker = session.get(format!("http://127.0.0.1:{slow}/")).send();
    let url = format!("http://localhost:{}/", spaced.addr().port());
    let spaced_calls = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        futures_util::future::join_all((0..3).map(|_| session.get(&url).send())).await
    };
    let (blocked, results) = tokio::join!(blocker, spaced_calls);
    blocked.unwrap();
    for result in results {
        result.unwrap();
    }
    let times = arrivals.lock().unwrap().clone();
    assert_eq!(times.len(), 3);
    for pair in times.windows(2) {
        assert!(
            pair[1] - pair[0] >= Duration::from_millis(200),
            "{:?}",
            pair[1] - pair[0]
        );
    }
}
