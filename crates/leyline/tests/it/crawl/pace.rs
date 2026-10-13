use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use leyline::HostLimits;

use super::*;

type Stamps = Arc<Mutex<Vec<(String, Instant)>>>;

fn stamp(stamps: &Stamps, target: &str) {
    stamps
        .lock()
        .unwrap()
        .push((target.to_owned(), Instant::now()));
}

fn at(stamps: &Stamps, target: &str) -> Instant {
    stamps
        .lock()
        .unwrap()
        .iter()
        .find(|(sent, _)| sent == target)
        .map(|(_, when)| *when)
        .unwrap()
}

#[tokio::test]
async fn a_crawl_keeps_crawl_delay_out_of_the_sessions_own_requests() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/robots.txt" => TestResponse::new(200).body("User-agent: *\nCrawl-delay: 3\n"),
        _ => html("<p>home</p>"),
    })
    .await
    .unwrap();
    let session = Session::builder()
        .host_limits(HostLimits::new().max_in_flight(4))
        .build()
        .unwrap();
    let mut crawl = session.crawl(server.url("/")).respect(Respect::Robots);
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    let started = Instant::now();
    for _ in 0..2 {
        session.get(server.url("/x")).send().await.unwrap();
    }
    assert!(started.elapsed() < Duration::from_millis(1500));
}

#[tokio::test]
async fn a_crawl_reports_invalid_host_limits() {
    let server = TestServer::http(|_| html("<p>home</p>")).await.unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .host_limits(HostLimits::new().pause_on([1000]));
    let error = crawl.next().await.unwrap().unwrap_err();
    assert_eq!(error.kind(), leyline::Kind::Config);
    assert!(crawl.next().await.is_none());
    assert!(targets(&server).await.is_empty());
}

#[tokio::test]
async fn a_crawl_tries_a_busy_page_again_and_gives_up_after_three() {
    let busy = Arc::new(AtomicUsize::new(0));
    let calls = busy.clone();
    let server = TestServer::http(move |request| match request.target.as_str() {
        "/" => html(r#"<a href="/a">A</a><a href="/b">B</a>"#),
        "/a" if calls.fetch_add(1, Ordering::SeqCst) == 0 => TestResponse::new(503),
        "/a" => html("<p>page a</p>"),
        "/b" => TestResponse::new(429),
        _ => TestResponse::new(404),
    })
    .await
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .host_limits(HostLimits::new());
    let mut kept = BTreeSet::new();
    while let Some(page) = crawl.next().await {
        kept.insert(page.unwrap().url().path().to_owned());
    }
    assert_eq!(kept, BTreeSet::from(["/", "/a"].map(String::from)));
    let sent = server.requests().await;
    let count = |path: &str| sent.iter().filter(|request| request.target == path).count();
    assert_eq!((count("/a"), count("/b")), (2, 3));
    assert_eq!(crawl.stats().skipped(SkipReason::Status), 1);
}

#[tokio::test]
async fn a_crawl_paces_a_session_that_limits_only_the_total() {
    let stamps = Stamps::default();
    let seen = stamps.clone();
    let server = TestServer::http(move |request| {
        stamp(&seen, &request.target);
        match request.target.as_str() {
            "/" => html(r#"<a href="/a">A</a>"#),
            _ => html("<p>a</p>"),
        }
    })
    .await
    .unwrap();
    let session = Session::builder()
        .host_limits(HostLimits::new().max_total_in_flight(100))
        .build()
        .unwrap();
    let mut crawl = session.crawl(server.url("/"));
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    let gap = at(&stamps, "/a") - at(&stamps, "/");
    assert!(gap >= Duration::from_millis(900), "{gap:?}");
}

#[tokio::test]
async fn a_paused_host_leaves_room_for_other_hosts() {
    let paused = TestServer::http(|_| TestResponse::new(503).header("retry-after", "3600"))
        .await
        .unwrap();
    let slow: String = (0..20)
        .map(|i| format!(r#"<a href="{}">S</a>"#, paused.url(&format!("/slow{i}"))))
        .collect();
    let server = TestServer::http(move |request| match request.target.as_str() {
        "/" => html(&format!(r#"{slow}<a href="/next">N</a>"#)),
        "/next" => html(r#"<p>next</p><a href="/fast">F</a>"#).delay(Duration::from_secs(1)),
        _ => html("<p>fast</p>"),
    })
    .await
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .host_limits(HostLimits::new().pause_on([503]));
    let reached = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(page) = crawl.next().await {
            if page.unwrap().url().path() == "/fast" {
                return true;
            }
        }
        false
    })
    .await;
    assert_eq!(reached, Ok(true));
}

#[tokio::test]
async fn a_crawl_with_its_own_limits_still_obeys_the_sessions_limits() {
    let stamps = Stamps::default();
    let seen = stamps.clone();
    let server = TestServer::http(move |request| {
        stamp(&seen, &request.target);
        match request.target.as_str() {
            "/" => html(r#"<a href="/a">A</a>"#),
            _ => html("<p>a</p>"),
        }
    })
    .await
    .unwrap();
    let session = Session::builder()
        .host_limits(HostLimits::new().per_second(1.0))
        .build()
        .unwrap();
    let mut crawl = session
        .crawl(server.url("/"))
        .host_limits(HostLimits::new());
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    let gap = at(&stamps, "/a") - at(&stamps, "/");
    assert!(gap >= Duration::from_millis(900), "{gap:?}");
}

#[tokio::test]
async fn a_crawl_waits_out_a_robots_txt_outage() {
    let started = Instant::now();
    let server = TestServer::http(move |request| match request.target.as_str() {
        "/robots.txt" if started.elapsed() < Duration::from_secs(2) => TestResponse::new(503),
        "/robots.txt" => TestResponse::new(200).body("User-agent: *\nDisallow: /never\n"),
        "/" => html(r#"<a href="/a">A</a>"#),
        _ => html("<p>a</p>"),
    })
    .await
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .respect(Respect::Robots)
        .host_limits(HostLimits::new());
    let mut kept = BTreeSet::new();
    while let Some(page) = crawl.next().await {
        kept.insert(page.unwrap().url().path().to_owned());
    }
    assert_eq!(kept, BTreeSet::from(["/", "/a"].map(String::from)));
}
