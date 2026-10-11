use super::*;

#[tokio::test]
async fn a_crawl_keeps_noindex_links_and_drops_copies() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/" => html(
            r#"<meta name="robots" content="noindex"><a href="/x">X</a><a href="/y">Y</a><a href="/z">Z</a><a href="/missing">M</a>"#,
        ),
        "/x" | "/y" => html("<p>the same text</p>"),
        "/z" => html(r#"<link rel="canonical" href="/x"><p>a printer view</p>"#),
        _ => TestResponse::new(404),
    })
    .await
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .respect(Respect::Robots);
    let mut kept = Vec::new();
    while let Some(page) = crawl.next().await {
        kept.push(page.unwrap().url().path().to_owned());
    }
    assert_eq!(kept, ["/x"]);
    let stats = crawl.stats();
    assert_eq!(stats.skipped(SkipReason::NoIndex), 1);
    assert_eq!(stats.skipped(SkipReason::Duplicate), 2);
    assert_eq!(stats.skipped(SkipReason::Status), 1);
}

#[tokio::test]
async fn a_crawl_follows_a_redirected_seed_to_its_new_host() {
    let target = TestServer::http(|request| match request.target.as_str() {
        "/start" => html(r#"<p>start</p><a href="/next">N</a>"#),
        "/next" => html("<p>next</p>"),
        _ => TestResponse::new(404),
    })
    .await
    .unwrap();
    let location = target.url("/start");
    let seed = TestServer::http(move |request| match request.target.as_str() {
        "/" => TestResponse::new(301).header("location", location.as_str()),
        _ => TestResponse::new(404),
    })
    .await
    .unwrap();
    let mut crawl = Session::new()
        .crawl(seed.url("/"))
        .respect(Respect::Robots)
        .scope(Scope::Host);
    let mut kept = Vec::new();
    while let Some(page) = crawl.next().await {
        kept.push(page.unwrap().url().path().to_owned());
    }
    assert_eq!(kept, ["/start", "/next"]);
    assert!(targets(&target).await.contains("/robots.txt"));
}

#[tokio::test]
async fn a_crawl_stops_at_the_page_budget_and_skips_files() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/" => html(
            r#"<a href="/report.pdf">R</a><a href="/1">1</a><a href="/2">2</a><a href="/3">3</a><a href="/4">4</a>"#,
        ),
        "/report.pdf" => TestResponse::new(200)
            .header("content-type", "application/pdf")
            .body("%PDF"),
        path => html(&format!("<p>page {path}</p>")),
    })
    .await
    .unwrap();
    let mut crawl = Session::new().crawl(server.url("/")).limit(3);
    let mut kept = 0;
    while let Some(page) = crawl.next().await {
        page.unwrap();
        kept += 1;
    }
    assert_eq!(kept, 3);
    assert_eq!(crawl.stats().skipped(SkipReason::NotHtml), 1);
    assert!(!targets(&server).await.contains("/4"));
}

#[tokio::test]
async fn a_shut_down_session_ends_the_crawl() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/" => html(r#"<a href="/slow1">1</a><a href="/slow2">2</a>"#),
        "/robots.txt" | "/sitemap.xml" => TestResponse::new(404),
        _ => html("<p>slow</p>").delay(Duration::from_secs(5)),
    })
    .await
    .unwrap();
    let session = Session::new();
    let mut crawl = session.crawl(server.url("/"));
    crawl.next().await.unwrap().unwrap();
    session.shutdown();
    let drained = tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(item) = crawl.next().await {
            assert!(item.unwrap_err().is_shut_down());
        }
    })
    .await;
    drained.expect("the crawl did not end after shutdown");
}

#[tokio::test]
async fn a_crawl_ignores_an_off_site_canonical_for_copies() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/" => html(r#"<a href="/one">1</a><a href="/two">2</a>"#),
        "/one" => {
            html(r#"<link rel="canonical" href="https://news.invalid/story"><p>first copy</p>"#)
        }
        "/two" => {
            html(r#"<link rel="canonical" href="https://news.invalid/story"><p>second, edited</p>"#)
        }
        _ => TestResponse::new(404),
    })
    .await
    .unwrap();
    let session = Session::builder()
        .host_limits(leyline::HostLimits::new().max_in_flight(4))
        .build()
        .unwrap();
    let mut crawl = session.crawl(server.url("/"));
    let mut pages = 0;
    while let Some(page) = crawl.next().await {
        page.unwrap();
        pages += 1;
    }
    assert_eq!(pages, 3);
}

#[tokio::test]
async fn a_crawl_sniffs_a_page_without_a_content_type() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/" => html(r#"<a href="/plain">P</a><a href="/data">D</a>"#),
        "/plain" => TestResponse::new(200).body("\n  <!doctype html><p>no header</p>"),
        "/data" => TestResponse::new(200).body("{\"a\": 1}"),
        _ => TestResponse::new(404),
    })
    .await
    .unwrap();
    let session = Session::builder()
        .host_limits(leyline::HostLimits::new().max_in_flight(4))
        .build()
        .unwrap();
    let mut crawl = session.crawl(server.url("/"));
    let mut kept = BTreeSet::new();
    while let Some(page) = crawl.next().await {
        kept.insert(page.unwrap().url().path().to_owned());
    }
    assert_eq!(kept, BTreeSet::from(["/".to_owned(), "/plain".to_owned()]));
    assert_eq!(crawl.stats().skipped(SkipReason::NotHtml), 1);
}
