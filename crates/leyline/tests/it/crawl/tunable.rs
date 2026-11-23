use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use leyline::{HostLimits, Kind, Redirect};

use super::*;

async fn collect_paths(crawl: &mut leyline::crawl::Crawl) -> Vec<String> {
    let mut paths = Vec::new();
    while let Some(page) = crawl.next().await {
        paths.push(page.expect("page succeeds").url().path().to_owned());
    }
    paths.sort();
    paths
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_crawl_runs_more_than_sixteen_pages_at_once_when_host_limits_allow_it() {
    let arrivals: Arc<Mutex<Vec<Instant>>> = Arc::default();
    let seen = Arc::clone(&arrivals);
    let links: String = (0..24)
        .map(|i| format!(r#"<a href="/p{i}">{i}</a>"#))
        .collect();
    let server = testing::Server::http(move |request| match request.target() {
        "/" => html(&links),
        path if path.starts_with("/p") => {
            seen.lock().expect("arrivals lock").push(Instant::now());
            html(&format!("<p>{path}</p>")).delay(Duration::from_millis(250))
        }
        _ => testing::Response::new(404),
    })
    .await
    .expect("start concurrency server");
    let session = Session::builder()
        .host_limits(HostLimits::new().max_in_flight(24).per_second(0.0))
        .build()
        .expect("build wide session");
    let mut crawl = session.crawl(server.url("/")).respect(None);
    while let Some(page) = crawl.next().await {
        page.expect("page succeeds");
    }
    let stamps = arrivals.lock().expect("arrivals lock").clone();
    let first = *stamps.first().expect("at least one page request");
    let overlapping = stamps
        .iter()
        .filter(|&&when| when.duration_since(first) < Duration::from_millis(180))
        .count();
    assert!(
        overlapping >= 20,
        "only {overlapping} requests overlapped before the first wave finished"
    );
}

#[tokio::test]
async fn a_zero_retry_session_does_not_retry_crawl_pages_or_robots() {
    let pages = testing::Server::http(|request| match request.target() {
        "/robots.txt" | "/sitemap.xml" => testing::Response::new(404),
        "/" => html(r#"<a href="/busy">B</a>"#),
        "/busy" => testing::Response::new(503),
        _ => testing::Response::new(404),
    })
    .await
    .expect("start page server");
    let session = Session::builder()
        .retry(0)
        .build()
        .expect("build zero-retry session");
    let mut crawl = session.crawl(pages.url("/")).host_limits(HostLimits::new());
    let _ = collect_paths(&mut crawl).await;
    let busy_hits = pages
        .requests()
        .await
        .iter()
        .filter(|req| req.target() == "/busy")
        .count();
    assert_eq!(busy_hits, 1);

    let down = testing::Server::http(|request| match request.target() {
        "/robots.txt" => testing::Response::new(503),
        _ => html("<p>page</p>"),
    })
    .await
    .expect("start robots server");
    let mut crawl = session.crawl(down.url("/")).host_limits(HostLimits::new());
    let _ = collect_paths(&mut crawl).await;
    let crawl_robots_hits = down
        .requests()
        .await
        .iter()
        .filter(|req| req.target() == "/robots.txt")
        .count();
    assert_eq!(crawl_robots_hits, 1);

    let direct = testing::Server::http(|_| testing::Response::new(503))
        .await
        .expect("start direct robots server");
    session
        .robots(direct.url("/"), "*")
        .await
        .expect("robots fallback succeeds");
    assert_eq!(direct.requests().await.len(), 1);
}

#[tokio::test]
async fn a_crawl_honors_the_sessions_redirect_policy() {
    let server = testing::Server::http(|request| {
        let target = request.target();
        if target == "/robots.txt" || target == "/sitemap.xml" {
            return testing::Response::new(404);
        }
        if target == "/" {
            return html(r#"<a href="/hop0">H</a>"#);
        }
        match target.strip_prefix("/hop") {
            Some("7") => html("<p>arrived at seven</p>"),
            Some(n) => {
                let next = n.parse::<u32>().expect("hop number") + 1;
                testing::Response::new(302).header("location", format!("/hop{next}"))
            }
            None => testing::Response::new(404),
        }
    })
    .await
    .expect("start redirect server");

    let none = Session::builder()
        .redirect(0)
        .build()
        .expect("build zero-redirect session");
    let mut crawl = none.crawl(server.url("/")).host_limits(HostLimits::new());
    assert_eq!(collect_paths(&mut crawl).await, ["/"]);
    assert_eq!(crawl.skipped(SkipReason::Redirects), 1);
    assert!(!targets(&server).await.contains("/hop1"));

    let deep = Session::builder()
        .redirect(8)
        .build()
        .expect("build eight-redirect session");
    let mut crawl = deep.crawl(server.url("/")).host_limits(HostLimits::new());
    assert_eq!(collect_paths(&mut crawl).await, ["/", "/hop7"]);
}

#[tokio::test]
async fn crawl_redirect_overrides_the_session_for_pages_and_sitemaps() {
    let server = testing::Server::http(|request| match request.target() {
        "/robots.txt" => testing::Response::new(404),
        "/sitemap.xml" => testing::Response::new(302).header("location", "/moved-map.xml"),
        "/moved-map.xml" => testing::Response::new(200)
            .header("content-type", "application/xml")
            .body("<urlset><url><loc>http://unused/from-map</loc></url></urlset>"),
        "/" => html(r#"<a href="/go">go</a><a href="/stop">stop</a>"#),
        "/go" => testing::Response::new(302).header("location", "/allowed"),
        "/stop" => testing::Response::new(302).header("location", "/blocked"),
        "/allowed" | "/blocked" => html("<p>target</p>"),
        _ => testing::Response::new(404),
    })
    .await
    .expect("start custom redirect server");

    let session = Session::builder()
        .redirect(8)
        .build()
        .expect("build permissive session");
    let mut crawl = session
        .crawl(server.url("/"))
        .host_limits(HostLimits::new())
        .redirect(Redirect::custom(|attempt| {
            attempt.location() == Some("/allowed")
        }));
    assert_eq!(collect_paths(&mut crawl).await, ["/", "/allowed"]);
    assert_eq!(crawl.skipped(SkipReason::Redirects), 2);
    assert!(!targets(&server).await.contains("/moved-map.xml"));
}

#[tokio::test]
async fn an_untyped_html_page_obeys_max_body_size() {
    let html_body = format!("<!doctype html><p>{}</p>", "x".repeat(2000));
    let json_body = format!(r#"{{"blob":"{}"}}"#, "y".repeat(2000));
    let server = testing::Server::http(move |request| match request.target() {
        "/robots.txt" | "/sitemap.xml" => testing::Response::new(404),
        "/" => html(r#"<a href="/bare">B</a><a href="/blob">J</a>"#),
        "/bare" => testing::Response::new(200).body(html_body.clone()),
        "/blob" => testing::Response::new(200).body(json_body.clone()),
        _ => testing::Response::new(404),
    })
    .await
    .expect("start untyped server");

    let mut capped = Session::new()
        .crawl(server.url("/"))
        .host_limits(HostLimits::new())
        .max_body_size(512);
    let mut kept = Vec::new();
    let mut errors = 0;
    while let Some(item) = capped.next().await {
        match item {
            Ok(page) => kept.push(page.url().path().to_owned()),
            Err(err) => {
                assert_eq!(err.kind(), Kind::BodyLimit);
                errors += 1;
            }
        }
    }
    assert_eq!(kept, ["/"]);
    assert_eq!(errors, 1);
    assert_eq!(capped.skipped(SkipReason::NotHtml), 1);
}

#[tokio::test]
async fn max_body_size_governs_robots_and_sitemaps() {
    let padding = "# ".to_owned() + &"p".repeat(600) + "\n";
    let huge_robots = format!("User-agent: *\n{}Disallow: /secret\n", padding.repeat(900));
    assert!(huge_robots.len() > 512 * 1024);
    let server = testing::Server::http(move |request| match request.target() {
        "/robots.txt" => testing::Response::new(200)
            .header("content-type", "text/plain")
            .body(huge_robots.clone()),
        "/sitemap.xml" => testing::Response::new(404),
        "/" => html(r#"<a href="/secret">S</a>"#),
        "/secret" => html("<p>secret</p>"),
        _ => testing::Response::new(404),
    })
    .await
    .expect("start large-robots server");

    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .host_limits(HostLimits::new())
        .max_body_size(1024 * 1024);
    assert_eq!(collect_paths(&mut crawl).await, ["/"]);
    assert_eq!(crawl.skipped(SkipReason::Robots), 1);

    let big_map = gzip(&format!(
        "<urlset><url><loc>{}</loc></url></urlset>",
        "https://unused.example/".to_owned() + &"a".repeat(2048)
    ));
    let map_server = testing::Server::http(move |request| match request.target() {
        "/robots.txt" => testing::Response::new(404),
        "/sitemap.xml" => testing::Response::new(200)
            .header("content-type", "application/gzip")
            .body(big_map.clone()),
        "/" => html("<p>home</p>"),
        _ => testing::Response::new(404),
    })
    .await
    .expect("start sitemap cap server");
    let mut capped = Session::new()
        .crawl(map_server.url("/"))
        .host_limits(HostLimits::new())
        .max_body_size(256);
    assert_eq!(collect_paths(&mut capped).await, ["/"]);
    assert_eq!(capped.errors(), 1);
}

#[tokio::test]
async fn limit_depth_url_size_and_sitemaps_are_tunable() {
    let long_path = format!("/{}", "x".repeat(2200));
    let long_for_server = long_path.clone();
    let server = testing::Server::http(move |request| match request.target() {
        "/robots.txt" => testing::Response::new(404),
        "/sitemap.xml" => testing::Response::new(200)
            .header("content-type", "application/xml")
            .body(format!(
                "<urlset><url><loc>http://{}/from-map</loc></url></urlset>",
                request.header("host").unwrap_or("127.0.0.1")
            )),
        "/" => html(&format!(
            r#"<a href="/d1">1</a><a href="{long_for_server}">L</a>"#
        )),
        "/d1" => html(r#"<a href="/d2">2</a>"#),
        "/d2" | "/from-map" => html("<p>leaf</p>"),
        path if path == long_for_server => html("<p>long</p>"),
        _ => testing::Response::new(404),
    })
    .await
    .expect("start bounds server");

    let mut shallow = Session::new()
        .crawl(server.url("/"))
        .host_limits(HostLimits::new())
        .limit(1)
        .limit(None)
        .max_sitemaps(0)
        .max_depth(1)
        .max_url_size(None);
    assert_eq!(
        collect_paths(&mut shallow).await,
        ["/", "/d1", long_path.as_str()]
    );
    assert!(!targets(&server).await.contains("/sitemap.xml"));
}

#[tokio::test]
async fn max_crawl_delay_and_robots_max_age_are_tunable() {
    let robots_reads = Arc::new(AtomicUsize::new(0));
    let reads = Arc::clone(&robots_reads);
    let server = testing::Server::http(move |request| match request.target() {
        "/robots.txt" => {
            let n = reads.fetch_add(1, Ordering::SeqCst);
            let body = if n == 0 {
                "User-agent: *\nCrawl-delay: 10\n"
            } else {
                "User-agent: *\nCrawl-delay: 10\nDisallow: /late\n"
            };
            testing::Response::new(200)
                .header("content-type", "text/plain")
                .body(body)
        }
        "/" => html(r#"<a href="/late">late</a>"#),
        "/late" => html("<p>late</p>"),
        _ => testing::Response::new(404),
    })
    .await
    .expect("start robots refresh server");

    let started = Instant::now();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .host_limits(HostLimits::new())
        .max_sitemaps(0)
        .max_crawl_delay(Duration::ZERO)
        .robots_max_age(Duration::ZERO);
    assert_eq!(collect_paths(&mut crawl).await, ["/"]);
    assert_eq!(crawl.skipped(SkipReason::Robots), 1);
    assert!(robots_reads.load(Ordering::SeqCst) >= 2);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn dedup_and_dedup_min_words_control_copy_filtering() {
    let twenty_five = (0..25)
        .map(|i| format!("w{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let four = "alpha beta gamma delta";
    let server = testing::Server::http(move |request| match request.target() {
        "/robots.txt" | "/sitemap.xml" => testing::Response::new(404),
        "/" => {
            html(r#"<a href="/a">a</a><a href="/b">b</a><a href="/s1">s1</a><a href="/s2">s2</a>"#)
        }
        "/a" => html(&format!(
            r#"<link rel="canonical" href="/b"><p>{twenty_five}</p>"#
        )),
        "/b" => html(&format!("<p>{twenty_five}</p>")),
        "/s1" | "/s2" => html(&format!("<p>{four}</p>")),
        _ => testing::Response::new(404),
    })
    .await
    .expect("start dedup server");

    let mut raw = Session::new()
        .crawl(server.url("/"))
        .host_limits(HostLimits::new())
        .dedup(false);
    assert_eq!(
        collect_paths(&mut raw).await,
        ["/", "/a", "/b", "/s1", "/s2"]
    );

    let mut strict = Session::new()
        .crawl(server.url("/"))
        .host_limits(HostLimits::new())
        .dedup_min_words(3);
    assert_eq!(collect_paths(&mut strict).await, ["/", "/b", "/s1"]);

    let mut canonical_only = Session::new()
        .crawl(server.url("/"))
        .host_limits(HostLimits::new())
        .dedup_min_words(None);
    assert_eq!(
        collect_paths(&mut canonical_only).await,
        ["/", "/b", "/s1", "/s2"]
    );
}

#[tokio::test]
async fn scope_any_follows_external_hosts_without_leaking_credentials() {
    let external = testing::Server::http(|request| match request.target() {
        "/robots.txt" | "/sitemap.xml" => testing::Response::new(404),
        "/ext" => {
            assert!(request.header("authorization").is_none());
            html("<p>external</p>")
        }
        _ => testing::Response::new(404),
    })
    .await
    .expect("start external server");
    let ext_url = format!("http://localhost:{}/ext", external.addr().port());
    let origin = testing::Server::http(move |request| match request.target() {
        "/robots.txt" | "/sitemap.xml" => testing::Response::new(404),
        "/" => {
            assert_eq!(request.header("authorization"), Some("Bearer secret"));
            html(&format!(r#"<a href="{ext_url}">ext</a>"#))
        }
        _ => testing::Response::new(404),
    })
    .await
    .expect("start origin server");

    let session = Session::builder()
        .bearer_auth("secret")
        .build()
        .expect("build authenticated session");
    let mut crawl = session
        .crawl(origin.url("/"))
        .scope(Scope::Any)
        .host_limits(HostLimits::new());
    assert_eq!(collect_paths(&mut crawl).await, ["/", "/ext"]);
}
