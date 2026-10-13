use std::sync::{Arc, Mutex};
use std::time::Instant;

use leyline::HostLimits;

use super::*;

type Stamps = Arc<Mutex<Vec<(String, Instant)>>>;

fn sent_at(stamps: &Stamps, target: &str) -> Instant {
    stamps
        .lock()
        .unwrap()
        .iter()
        .find(|(sent, _)| sent == target)
        .map(|(_, when)| *when)
        .unwrap()
}

#[tokio::test]
async fn a_crawl_keeps_its_spacing_after_waiting_for_the_sessions_slot() {
    let stamps = Stamps::default();
    let seen = stamps.clone();
    let server = TestServer::http(move |request| {
        seen.lock()
            .unwrap()
            .push((request.target.clone(), Instant::now()));
        match request.target.as_str() {
            "/hold" => html("<p>hold</p>").delay(Duration::from_secs(2)),
            "/" => html(r#"<a href="/p1">1</a><a href="/p2">2</a>"#),
            _ => html("<p>page</p>"),
        }
    })
    .await
    .unwrap();
    let session = Session::builder()
        .host_limits(HostLimits::new().max_in_flight(1))
        .build()
        .unwrap();
    let mut crawl = session
        .crawl(server.url("/"))
        .ignore_robots()
        .host_limits(HostLimits::new().max_in_flight(2).per_second(2.0));
    crawl.next().await.unwrap().unwrap();
    let holder = session.clone();
    let hold_url = server.url("/hold");
    let hold = tokio::spawn(async move { holder.get(hold_url).send().await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    hold.await.unwrap().unwrap();
    let gap = sent_at(&stamps, "/p2").max(sent_at(&stamps, "/p1"))
        - sent_at(&stamps, "/p2").min(sent_at(&stamps, "/p1"));
    assert!(gap >= Duration::from_millis(400), "{gap:?}");
}

#[tokio::test]
async fn hosts_asking_to_retry_later_leave_room_for_other_hosts() {
    let mut busy = Vec::new();
    for _ in 0..20 {
        let server = TestServer::http(|request| match request.target.as_str() {
            "/robots.txt" => TestResponse::new(404),
            _ => TestResponse::new(429).header("retry-after", "30"),
        })
        .await
        .unwrap();
        busy.push(server);
    }
    let links: String = busy
        .iter()
        .map(|server| format!(r#"<a href="{}">S</a>"#, server.url("/slow")))
        .collect();
    let server = TestServer::http(move |request| match request.target.as_str() {
        "/" => html(&format!(r#"{links}<a href="/next">N</a>"#)),
        "/next" => html(r#"<p>next</p><a href="/fast">F</a>"#).delay(Duration::from_secs(2)),
        _ => html("<p>fast</p>"),
    })
    .await
    .unwrap();
    let mut crawl = Session::new().crawl(server.url("/"));
    let reached = tokio::time::timeout(Duration::from_secs(12), async {
        while let Some(page) = crawl.next().await {
            if page.is_ok_and(|page| page.url().path() == "/fast") {
                return true;
            }
        }
        false
    })
    .await;
    assert_eq!(reached, Ok(true));
}

#[tokio::test]
async fn an_ignore_robots_crawl_does_not_wait_out_a_robots_txt_pause() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/robots.txt" => TestResponse::new(503).header("retry-after", "3600"),
        "/" => html(r#"<a href="/a">A</a>"#),
        _ => html("<p>a</p>"),
    })
    .await
    .unwrap();
    let mut crawl = Session::new().crawl(server.url("/")).ignore_robots();
    let pages = tokio::time::timeout(Duration::from_secs(8), async {
        let mut pages = 0;
        while let Some(page) = crawl.next().await {
            page.unwrap();
            pages += 1;
        }
        pages
    })
    .await;
    assert_eq!(pages, Ok(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shut_down_session_ends_a_crawl_waiting_on_a_paused_host() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/" => html(r#"<a href="/a">A</a><a href="/b">B</a>"#),
        "/a" | "/b" => TestResponse::new(429).header("retry-after", "3600"),
        _ => TestResponse::new(404),
    })
    .await
    .unwrap();
    let session = Session::new();
    let crawl = session.crawl(server.url("/"));
    let run = tokio::spawn(async move {
        let mut crawl = crawl;
        while crawl.next().await.is_some() {}
    });
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(!run.is_finished());
    let stopped = Instant::now();
    session.shutdown();
    run.await.unwrap();
    assert!(
        stopped.elapsed() < Duration::from_secs(1),
        "{:?}",
        stopped.elapsed()
    );
}

#[tokio::test]
async fn a_host_gate_goes_away_once_its_requests_are_done() {
    let first = TestServer::http(|_| html("<p>a</p>")).await.unwrap();
    let second = TestServer::http(|_| html("<p>b</p>")).await.unwrap();
    let session = Session::builder()
        .host_limits(HostLimits::new().max_in_flight(1))
        .build()
        .unwrap();
    session.get(first.url("/")).send().await.unwrap();
    session.get(second.url("/")).send().await.unwrap();
    assert_eq!(session.host_stats().len(), 1);
}

#[tokio::test]
async fn a_crawl_keeps_the_sessions_spacing_while_it_waits_for_its_own_slot() {
    let stamps = Stamps::default();
    let seen = stamps.clone();
    let server = TestServer::http(move |request| {
        seen.lock()
            .unwrap()
            .push((request.target.clone(), Instant::now()));
        match request.target.as_str() {
            "/" => html(r#"<a href="/hold">H</a><a href="/p1">1</a><a href="/p2">2</a>"#),
            "/hold" => html("<p>hold</p>").delay(Duration::from_secs(3)),
            _ => html("<p>page</p>"),
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
        .ignore_robots()
        .host_limits(HostLimits::new().max_total_in_flight(1));
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    let gap = sent_at(&stamps, "/p2").max(sent_at(&stamps, "/p1"))
        - sent_at(&stamps, "/p2").min(sent_at(&stamps, "/p1"));
    assert!(gap >= Duration::from_millis(900), "{gap:?}");
}

#[tokio::test]
async fn a_crawl_keeps_a_sites_crawl_delay_while_it_visits_other_hosts() {
    let stamps = Stamps::default();
    let seen = stamps.clone();
    let mut others = Vec::new();
    for _ in 0..6 {
        let other = TestServer::http(|request| match request.target.as_str() {
            "/robots.txt" => TestResponse::new(404),
            _ => html("<p>other</p>"),
        })
        .await
        .unwrap();
        others.push(other);
    }
    let links: String = others
        .iter()
        .map(|other| format!(r#"<a href="{}">O</a>"#, other.url("/o")))
        .collect();
    let server = TestServer::http(move |request| {
        seen.lock()
            .unwrap()
            .push((request.target.clone(), Instant::now()));
        match request.target.as_str() {
            "/robots.txt" => TestResponse::new(200).body("User-agent: *\nCrawl-delay: 3\n"),
            "/" => html(&format!(r#"{links}<a href="/a2">A</a>"#)),
            _ => html("<p>a</p>"),
        }
    })
    .await
    .unwrap();
    let mut crawl = Session::new().crawl(server.url("/"));
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    let gap = sent_at(&stamps, "/a2") - sent_at(&stamps, "/");
    assert!(gap >= Duration::from_millis(2700), "{gap:?}");
}

#[tokio::test]
async fn a_crawl_retries_a_page_within_the_sessions_retry_budget() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/" => html(r#"<a href="/busy">B</a>"#),
        "/busy" => TestResponse::new(503),
        _ => TestResponse::new(404),
    })
    .await
    .unwrap();
    let session = Session::builder()
        .retry(leyline::RetryPolicy::transient().max_retries(1))
        .build()
        .unwrap();
    let mut crawl = session
        .crawl(server.url("/"))
        .host_limits(HostLimits::new());
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    let busy = server
        .requests()
        .await
        .iter()
        .filter(|request| request.target == "/busy")
        .count();
    assert_eq!(busy, 2);
}

#[tokio::test]
async fn an_ignore_robots_crawl_never_asks_for_robots_txt() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/" => html(r#"<a href="/a">A</a>"#),
        _ => html("<p>a</p>"),
    })
    .await
    .unwrap();
    let mut crawl = Session::new().crawl(server.url("/")).ignore_robots();
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    assert!(!targets(&server).await.contains("/robots.txt"));
}
