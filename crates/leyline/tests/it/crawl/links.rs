use super::*;

#[tokio::test]
async fn a_crawl_stops_at_the_page_budget_and_skips_files() {
    let server = testing::Server::http(|request| match request.target.as_str() {
        "/" => html(
            r#"<a href="/report.pdf">R</a><a href="/1">1</a><a href="/2">2</a><a href="/3">3</a><a href="/4">4</a>"#,
        ),
        "/report.pdf" => testing::Response::new(200)
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
    assert_eq!(crawl.skipped(SkipReason::NotHtml), 1);
    assert!(!targets(&server).await.contains("/4"));
}

#[tokio::test]
async fn a_shut_down_session_ends_the_crawl() {
    let server = testing::Server::http(|request| match request.target.as_str() {
        "/" => html(r#"<a href="/slow1">1</a><a href="/slow2">2</a>"#),
        "/robots.txt" | "/sitemap.xml" => testing::Response::new(404),
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
async fn a_crawl_sniffs_a_page_without_a_content_type() {
    let server = testing::Server::http(|request| match request.target.as_str() {
        "/" => html(r#"<a href="/plain">P</a><a href="/data">D</a>"#),
        "/plain" => testing::Response::new(200).body("\n  <!doctype html><p>no header</p>"),
        "/data" => testing::Response::new(200).body("{\"a\": 1}"),
        _ => testing::Response::new(404),
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
    assert_eq!(crawl.skipped(SkipReason::NotHtml), 1);
}

#[tokio::test]
async fn a_crawl_resolves_links_against_the_base_element() {
    let server = testing::Server::http(|request| match request.target.as_str() {
        "/" => html(r#"<base href="/docs/"><a href="guide">G</a>"#),
        "/docs/guide" => html("<p>guide</p>"),
        _ => testing::Response::new(404),
    })
    .await
    .unwrap();
    let mut crawl = Session::new().crawl(server.url("/"));
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    assert_eq!(
        page_targets(&server).await,
        BTreeSet::from(["/", "/docs/guide"].map(String::from))
    );
}

#[tokio::test]
async fn a_crawl_sniffs_html_that_breaks_its_first_tag_with_a_newline() {
    let server = testing::Server::http(|request| match request.target.as_str() {
        "/" => html(r#"<a href="/bare">B</a>"#),
        _ => testing::Response::new(200).body("<html\n><p>no content type</p></html>"),
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
    assert!(kept.contains("/bare"), "{kept:?}");
}

#[tokio::test]
async fn a_crawl_sniffs_html_that_starts_with_a_comment() {
    let server = testing::Server::http(|request| match request.target.as_str() {
        "/" => html(r#"<a href="/bare">B</a>"#),
        _ => testing::Response::new(200).body("<!--generated--><p>no content type</p>"),
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
    assert!(kept.contains("/bare"), "{kept:?}");
}
