use super::*;

#[tokio::test]
async fn a_crawl_that_respects_robots_obeys_robots_txt_and_nofollow() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/robots.txt" => TestResponse::new(200).body("User-agent: *\nDisallow: /private\n"),
        "/" => html(
            r#"<a href="/a#top">A</a> <a href="/private/x">P</a>
               <a href="/b" rel="nofollow">B</a> <a href="http://other.invalid/">O</a>
               <a href="mailto:me@example.com">M</a>"#,
        ),
        "/a" => html(r#"<p>page a</p><a href="/c">C</a>"#),
        "/c" => html(r#"<p>page c</p><a href="/d">D</a>"#),
        _ => html("<p>other</p>"),
    })
    .await
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .respect(Respect::Robots);
    let mut kept = Vec::new();
    while let Some(page) = crawl.next().await {
        let page = page.unwrap();
        kept.push((page.url().path().to_owned(), page.depth()));
    }
    assert_eq!(
        kept,
        [
            ("/".to_owned(), 0),
            ("/a".to_owned(), 1),
            ("/c".to_owned(), 2),
            ("/d".to_owned(), 3)
        ]
    );
    assert_eq!(
        page_targets(&server).await,
        BTreeSet::from(["/", "/a", "/c", "/d"].map(String::from))
    );
    assert_eq!(crawl.stats().skipped(SkipReason::Robots), 1);
    assert_eq!(crawl.stats().pages(), 4);
}

#[tokio::test]
async fn a_crawl_ignores_robots_rules_when_told() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/robots.txt" => TestResponse::new(200).body("User-agent: *\nDisallow: /\n"),
        "/" => html(
            r#"<meta name="robots" content="noindex, nofollow"><a href="/private" rel="nofollow">P</a>"#,
        ),
        _ => html("<p>private</p>"),
    })
    .await
    .unwrap();
    let mut crawl = Session::new().crawl(server.url("/")).ignore_robots();
    let mut pages = 0;
    while let Some(page) = crawl.next().await {
        page.unwrap();
        pages += 1;
    }
    assert_eq!(pages, 2);
    assert_eq!(
        page_targets(&server).await,
        BTreeSet::from(["/", "/private"].map(String::from))
    );
}

#[tokio::test]
async fn a_crawl_for_ai_training_obeys_content_signals() {
    let handler = |request: &leyline::testing::RecordedRequest| match request.target.as_str() {
        "/robots.txt" => {
            TestResponse::new(200).body("User-agent: *\nContent-Signal: search=yes, ai-train=no\n")
        }
        _ => html("<p>home</p>"),
    };
    let server = TestServer::http(handler).await.unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .respect(Respect::AiTraining);
    assert!(crawl.next().await.is_none());
    assert_eq!(crawl.stats().skipped(SkipReason::Signal), 1);
    assert!(page_targets(&server).await.is_empty());

    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .respect(Respect::Robots);
    crawl.next().await.unwrap().unwrap();
}

#[tokio::test]
async fn a_crawl_reads_sitemaps_from_robots_and_indexes() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let robots = format!("User-agent: *\nDisallow: /hidden\nSitemap: {base}/sitemap_index.xml\n");
    let index = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><sitemapindex><sitemap><loc>{base}/pages.xml.gz</loc></sitemap></sitemapindex>"#
    );
    let pages = gzip(&format!(
        "<urlset><url><loc>{base}/orphan?a=1&amp;b=2</loc></url><url><loc>{base}/hidden</loc></url><url><loc>http://other.invalid/x</loc></url></urlset>"
    ));
    let server = TestServer::http_on(listener, move |request| match request.target.as_str() {
        "/robots.txt" => TestResponse::new(200).body(robots.clone()),
        "/sitemap_index.xml" => TestResponse::new(200)
            .header("content-type", "application/xml")
            .body(index.clone()),
        "/pages.xml.gz" => TestResponse::new(200)
            .header("content-type", "application/gzip")
            .body(pages.clone()),
        "/" => html("<p>home, no links</p>"),
        "/orphan?a=1&b=2" => html("<p>a page only the sitemap lists</p>"),
        _ => TestResponse::new(404),
    })
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .respect(Respect::Robots);
    let mut kept = BTreeSet::new();
    while let Some(page) = crawl.next().await {
        let page = page.unwrap();
        kept.insert(format!(
            "{}{}",
            page.url().path(),
            page.url()
                .query()
                .map(|q| format!("?{q}"))
                .unwrap_or_default()
        ));
    }
    assert_eq!(
        kept,
        BTreeSet::from(["/".to_owned(), "/orphan?a=1&b=2".to_owned()])
    );
    let sent = targets(&server).await;
    assert!(sent.contains("/pages.xml.gz"), "{sent:?}");
    assert!(!sent.contains("/hidden"), "{sent:?}");
    assert_eq!(crawl.stats().skipped(SkipReason::Robots), 1);
}

#[tokio::test]
async fn a_crawl_falls_back_to_sitemap_xml() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let sitemap = format!("<urlset><url><loc>{base}/orphan</loc></url></urlset>");
    let server = TestServer::http_on(listener, move |request| match request.target.as_str() {
        "/sitemap.xml" => TestResponse::new(200).body(sitemap.clone()),
        "/" => html("<p>home</p>"),
        "/orphan" => html("<p>orphan</p>"),
        _ => TestResponse::new(404),
    })
    .unwrap();
    let mut crawl = Session::new().crawl(server.url("/"));
    let mut pages = 0;
    while let Some(page) = crawl.next().await {
        page.unwrap();
        pages += 1;
    }
    assert_eq!(pages, 2);
}

#[tokio::test]
async fn a_crawl_applies_the_sites_crawl_delay() {
    let stamps = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = stamps.clone();
    let server = TestServer::http(move |request| match request.target.as_str() {
        "/robots.txt" => TestResponse::new(200).body("User-agent: *\nCrawl-delay: 2\n"),
        path => {
            seen.lock().unwrap().push(std::time::Instant::now());
            match path {
                "/" => html(r#"<a href="/a">A</a>"#),
                _ => html("<p>a</p>"),
            }
        }
    })
    .await
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .respect(Respect::Robots);
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    let stamps = stamps.lock().unwrap().clone();
    assert_eq!(stamps.len(), 3);
    assert!(
        stamps[1] - stamps[0] >= Duration::from_millis(1900),
        "{:?}",
        stamps[1] - stamps[0]
    );
}

#[tokio::test]
async fn a_crawl_reads_robots_txt_again_after_a_day() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/robots.txt" => TestResponse::new(200).body("User-agent: *\nDisallow: /never\n"),
        "/" => html(r#"<a href="/a">A</a>"#),
        "/a" => html(r#"<p>a</p><a href="/b">B</a>"#),
        _ => html("<p>b</p>"),
    })
    .await
    .unwrap();
    let session = Session::builder()
        .host_limits(leyline::HostLimits::new().max_in_flight(4))
        .build()
        .unwrap();
    let mut crawl = session.crawl(server.url("/")).respect(Respect::Robots);
    crawl.next().await.unwrap().unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(25 * 60 * 60)).await;
    tokio::time::resume();
    let mut rest = 0;
    while let Some(page) = crawl.next().await {
        page.unwrap();
        rest += 1;
    }
    assert_eq!(rest, 2);
    let robots = server
        .requests()
        .await
        .iter()
        .filter(|request| request.target == "/robots.txt")
        .count();
    assert_eq!(robots, 2);
}

#[tokio::test]
async fn a_crawl_for_ai_training_obeys_content_usage_by_path_and_header() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/robots.txt" => {
            TestResponse::new(200).body("User-agent: *\nContent-Usage: /closed/ train-ai=n\n")
        }
        "/" => html(r#"<a href="/open">O</a><a href="/closed/x">C</a><a href="/header">H</a>"#),
        "/header" => html("<p>header says no</p>").header("content-usage", "train-ai=n"),
        path => html(&format!("<p>{path}</p>")),
    })
    .await
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .host_limits(leyline::HostLimits::new())
        .respect(Respect::AiTraining);
    let mut kept = BTreeSet::new();
    while let Some(page) = crawl.next().await {
        kept.insert(page.unwrap().url().path().to_owned());
    }
    assert_eq!(kept, BTreeSet::from(["/".to_owned(), "/open".to_owned()]));
    assert!(!page_targets(&server).await.contains("/closed/x"));
    assert_eq!(crawl.stats().skipped(SkipReason::Signal), 2);
}

#[tokio::test]
async fn a_crawl_obeys_robots_rules_by_default() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/robots.txt" => TestResponse::new(200).body("User-agent: *\nDisallow: /private\n"),
        "/" => html(r#"<a href="/private">P</a><a href="/open">O</a><a href="/hidden">H</a>"#),
        "/hidden" => html(r#"<meta name="robots" content="noindex"><p>hidden</p>"#),
        _ => html("<p>page</p>"),
    })
    .await
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .host_limits(leyline::HostLimits::new());
    let mut kept = BTreeSet::new();
    while let Some(page) = crawl.next().await {
        kept.insert(page.unwrap().url().path().to_owned());
    }
    assert_eq!(kept, BTreeSet::from(["/", "/open"].map(String::from)));
    assert!(!targets(&server).await.contains("/private"));
}

#[tokio::test]
async fn a_crawl_retries_a_robots_txt_that_fails_with_500() {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = calls.clone();
    let server = TestServer::http(move |request| match request.target.as_str() {
        "/robots.txt" if count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 => {
            TestResponse::new(500)
        }
        "/robots.txt" => TestResponse::new(200).body("User-agent: *\nDisallow: /never\n"),
        "/" => html(r#"<a href="/a">A</a>"#),
        _ => html("<p>a</p>"),
    })
    .await
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .host_limits(leyline::HostLimits::new());
    let mut pages = 0;
    while let Some(page) = crawl.next().await {
        page.unwrap();
        pages += 1;
    }
    assert_eq!(pages, 2);
}
