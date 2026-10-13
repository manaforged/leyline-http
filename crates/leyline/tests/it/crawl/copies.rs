use super::*;

#[tokio::test]
async fn a_crawl_keeps_noindex_links_and_drops_copies() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/" => html(
            r#"<meta name="robots" content="noindex"><a href="/x">X</a><a href="/y">Y</a><a href="/z">Z</a><a href="/missing">M</a>"#,
        ),
        "/x" | "/y" => html(
            "<p>the same text on two pages, long enough that the crawl compares it word by word and keeps only the first copy it finds</p>",
        ),
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
async fn a_crawl_keeps_a_page_whose_text_matched_only_a_skipped_copy() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/" => html(r#"<a href="/x">X</a><a href="/z">Z</a><a href="/w">W</a>"#),
        "/x" => html("<p>the original article text that is long enough for the crawl to compare it word by word with every other page it keeps</p>"),
        "/z" => html(r#"<link rel="canonical" href="/x"><p>a different article text that is also long enough for the crawl to compare it word by word with every page it keeps</p>"#),
        "/w" => html("<p>a different article text that is also long enough for the crawl to compare it word by word with every page it keeps</p>"),
        _ => TestResponse::new(404),
    })
    .await
    .unwrap();
    let mut crawl = Session::new().crawl(server.url("/"));
    let mut kept = BTreeSet::new();
    while let Some(page) = crawl.next().await {
        kept.insert(page.unwrap().url().path().to_owned());
    }
    assert_eq!(kept, BTreeSet::from(["/", "/x", "/w"].map(String::from)));
}

#[tokio::test]
async fn a_crawl_keeps_the_canonical_page_over_an_alias_found_first() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/" => html(r#"<a href="/alias">A</a>"#),
        "/alias" => html(r#"<link rel="canonical" href="/real"><p>the article</p>"#),
        "/real" => html(r#"<link rel="canonical" href="/real"><p>the article</p>"#),
        _ => TestResponse::new(404),
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
    assert_eq!(kept, BTreeSet::from(["/", "/real"].map(String::from)));
}

#[tokio::test]
async fn a_crawl_keeps_an_alias_whose_canonical_page_is_gone() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/" => html(r#"<a href="/alias">A</a>"#),
        "/alias" => html(r#"<link rel="canonical" href="/real"><p>the article</p>"#),
        _ => TestResponse::new(404),
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
    assert_eq!(kept, BTreeSet::from(["/", "/alias"].map(String::from)));
}

#[tokio::test]
async fn a_crawl_keeps_short_pages_that_share_their_text() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/" => html(r#"<p>home page</p><a href="/a">A</a><a href="/b">B</a>"#),
        _ => html("<p>Please enable JavaScript.</p>"),
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
    assert_eq!(kept, BTreeSet::from(["/", "/a", "/b"].map(String::from)));
}

#[tokio::test]
async fn a_crawl_follows_a_chain_of_canonical_pages_to_its_end() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/" => html(r#"<a href="/a">A</a>"#),
        "/a" => html(r#"<link rel="canonical" href="/b"><p>story</p>"#),
        "/b" => html(r#"<link rel="canonical" href="/c"><p>story</p>"#),
        "/c" => html(r#"<link rel="canonical" href="/c"><p>story</p>"#),
        _ => TestResponse::new(404),
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
    assert_eq!(kept, BTreeSet::from(["/", "/c"].map(String::from)));
}

#[tokio::test]
async fn a_crawl_holds_an_alias_through_its_canonical_pages_redirect() {
    let text = "<p>the one article that the alias, the canonical address, and its new home all carry word for word today</p>";
    let alias = format!(r#"<link rel="canonical" href="/canonical">{text}"#);
    let actual = format!(r#"<link rel="canonical" href="/actual">{text}"#);
    let server = TestServer::http(move |request| match request.target.as_str() {
        "/" => html(r#"<a href="/alias">A</a>"#),
        "/alias" => html(&alias),
        "/canonical" => TestResponse::new(301).header("location", "/actual"),
        "/actual" => html(&actual),
        _ => TestResponse::new(404),
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
    assert_eq!(kept, BTreeSet::from(["/", "/actual"].map(String::from)));
}

#[tokio::test]
async fn held_aliases_count_toward_the_page_limit() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/" => html(r#"<a href="/a1">1</a><a href="/a2">2</a><a href="/c">C</a>"#),
        "/a1" => html(r#"<link rel="canonical" href="/b1"><p>first alias</p>"#),
        "/a2" => html(r#"<link rel="canonical" href="/b2"><p>second alias</p>"#),
        "/c" => html("<p>another page</p>"),
        _ => TestResponse::new(404),
    })
    .await
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .limit(3)
        .host_limits(leyline::HostLimits::new());
    let mut pages = 0;
    while let Some(page) = crawl.next().await {
        page.unwrap();
        pages += 1;
    }
    assert_eq!(pages, 3);
    assert_eq!(crawl.stats().requests(), 3);
}
