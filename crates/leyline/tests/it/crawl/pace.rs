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
async fn paused_hosts_leave_room_for_other_hosts() {
    let mut paused = Vec::new();
    for _ in 0..40 {
        let server = TestServer::http(|request| match request.target.as_str() {
            "/robots.txt" => TestResponse::new(404),
            _ => TestResponse::new(503).header("retry-after", "3600"),
        })
        .await
        .unwrap();
        paused.push(server);
    }
    let slow: String = paused
        .iter()
        .map(|server| {
            format!(
                r#"<a href="{}">S</a><a href="{}">S</a>"#,
                server.url("/slow"),
                server.url("/slow2")
            )
        })
        .collect();
    let server = TestServer::http(move |request| match request.target.as_str() {
        "/" => html(&format!(r#"{slow}<a href="/next">N</a>"#)),
        "/next" => html(r#"<p>next</p><a href="/fast">F</a>"#).delay(Duration::from_secs(3)),
        _ => html("<p>fast</p>"),
    })
    .await
    .unwrap();
    let mut crawl = Session::new().crawl(server.url("/"));
    let reached = tokio::time::timeout(Duration::from_secs(20), async {
        while let Some(page) = crawl.next().await {
            if page.is_ok_and(|page| page.url().path() == "/fast") {
                return true;
            }
        }
        false
    })
    .await;
    assert_eq!(reached, Ok(true));
    let mut hit = 0;
    for server in &paused {
        hit += usize::from(targets(server).await.contains("/slow"));
    }
    assert!(hit >= 17, "{hit} paused hosts were asked for a page");
}

#[tokio::test]
async fn crawls_of_one_session_keep_each_others_crawl_delay() {
    let stamps = Stamps::default();
    let seen = stamps.clone();
    let server = TestServer::http(move |request| {
        stamp(&seen, &request.target);
        match request.target.as_str() {
            "/robots.txt" => TestResponse::new(200).body("User-agent: *\nCrawl-delay: 3\n"),
            "/" => html(r#"<a href="/a2">A</a>"#),
            "/b" => html(
                r#"<a href="/b2">B</a><a href="/b3">B</a><a href="/b4">B</a><a href="/b5">B</a><a href="/b6">B</a>"#,
            ),
            _ => html("<p>page</p>"),
        }
    })
    .await
    .unwrap();
    let session = Session::new();
    let polite = session.crawl(server.url("/"));
    let fast = session.crawl(server.url("/b")).ignore_robots();
    let drain = |mut crawl: leyline::Crawl| async move {
        while let Some(page) = crawl.next().await {
            page.unwrap();
        }
    };
    tokio::join!(drain(fast), drain(polite));
    let polite_at = at(&stamps, "/a2");
    let before = stamps
        .lock()
        .unwrap()
        .iter()
        .filter(|(target, when)| target != "/robots.txt" && *when < polite_at)
        .map(|(_, when)| *when)
        .max()
        .unwrap();
    let gap = polite_at - before;
    assert!(gap >= Duration::from_millis(2700), "{gap:?}");
}

#[tokio::test]
async fn a_finished_crawl_releases_the_sessions_host_slot() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let local = listener.local_addr().unwrap();
    let port = local.port();
    let robots = format!("User-agent: *\nSitemap: http://maps.example.test:{port}/map.xml\n");
    let _server = TestServer::http_on(listener, move |request| match request.target.as_str() {
        "/robots.txt" => TestResponse::new(200).body(robots.clone()),
        "/map.xml" => TestResponse::new(200)
            .body("<urlset></urlset>")
            .delay(Duration::from_secs(10)),
        "/" => html(r#"<p>home</p><a href="/a">A</a>"#).delay(Duration::from_secs(1)),
        _ => html("<p>a</p>").delay(Duration::from_secs(1)),
    })
    .unwrap();
    let session = Session::builder()
        .dns(
            leyline::DnsConfig::new()
                .resolve_host("www.example.test", [local])
                .resolve_host("maps.example.test", [local]),
        )
        .host_limits(HostLimits::new().max_in_flight(1))
        .build()
        .unwrap();
    let mut crawl = session
        .crawl(format!("http://www.example.test:{port}/"))
        .limit(2)
        .host_limits(HostLimits::new());
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    let started = Instant::now();
    session
        .get(format!("http://maps.example.test:{port}/after"))
        .send()
        .await
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    drop(crawl);
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

#[tokio::test]
async fn a_crawl_paces_a_host_the_session_overrides() {
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
        .host_limits(HostLimits::new().host("127.0.0.1", HostLimits::new().max_in_flight(8)))
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
async fn a_crawl_that_ignores_robots_ignores_an_earlier_crawl_delay() {
    let stamps = Stamps::default();
    let seen = stamps.clone();
    let server = TestServer::http(move |request| {
        stamp(&seen, &request.target);
        match request.target.as_str() {
            "/robots.txt" => TestResponse::new(200).body("User-agent: *\nCrawl-delay: 4\n"),
            "/" | "/again" => html(r#"<a href="/b">B</a>"#),
            _ => html("<p>b</p>"),
        }
    })
    .await
    .unwrap();
    let session = Session::new();
    let mut first = session.crawl(server.url("/")).limit(1);
    while let Some(page) = first.next().await {
        page.unwrap();
    }
    let mut second = session.crawl(server.url("/again")).ignore_robots();
    while let Some(page) = second.next().await {
        page.unwrap();
    }
    let gap = at(&stamps, "/b") - at(&stamps, "/again");
    assert!(gap < Duration::from_millis(2500), "{gap:?}");
}

#[tokio::test]
async fn a_crawl_that_ignores_robots_does_not_wait_for_robots_txt() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/robots.txt" => TestResponse::new(503),
        "/" => html(r#"<a href="/a">A</a>"#),
        _ => html("<p>a</p>"),
    })
    .await
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .ignore_robots()
        .host_limits(HostLimits::new());
    let started = Instant::now();
    let mut pages = 0;
    while let Some(page) = crawl.next().await {
        page.unwrap();
        pages += 1;
    }
    assert_eq!(pages, 2);
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "{:?}",
        started.elapsed()
    );
}
