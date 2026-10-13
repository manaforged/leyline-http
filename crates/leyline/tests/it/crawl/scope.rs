use super::*;

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
async fn a_crawl_keeps_a_page_that_redirects_back_to_itself_through_a_check() {
    let visits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = visits.clone();
    let server = TestServer::http(move |request| match request.target.as_str() {
        "/" => html(r#"<a href="/article">A</a>"#),
        "/article" if count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 => {
            TestResponse::new(302).header("location", "/check")
        }
        "/article" => html("<p>article</p>"),
        "/check" => TestResponse::new(302).header("location", "/article"),
        _ => TestResponse::new(404),
    })
    .await
    .unwrap();
    let mut crawl = Session::new().crawl(server.url("/"));
    let mut kept = BTreeSet::new();
    while let Some(page) = crawl.next().await {
        kept.insert(page.unwrap().url().path().to_owned());
    }
    assert_eq!(kept, BTreeSet::from(["/", "/article"].map(String::from)));
}

#[tokio::test]
async fn a_domain_crawl_from_an_address_stays_on_that_address() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/" => html(r#"<a href="http://127.1.0.1:9/x">X</a><a href="/y">Y</a>"#),
        _ => html("<p>y</p>"),
    })
    .await
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .scope(Scope::Domain)
        .host_limits(leyline::HostLimits::new());
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    assert_eq!(crawl.stats().requests(), 2);
}

#[tokio::test]
async fn a_seed_redirected_to_another_site_yields_only_that_page() {
    let target = TestServer::http(|request| match request.target.as_str() {
        "/start" => html(r#"<p>start</p><a href="/next">N</a>"#),
        _ => html("<p>next</p>"),
    })
    .await
    .unwrap();
    let location = format!("http://localhost:{}/start", target.addr().port());
    let seed =
        TestServer::http(move |_| TestResponse::new(301).header("location", location.as_str()))
            .await
            .unwrap();
    let mut crawl = Session::new()
        .crawl(seed.url("/"))
        .host_limits(leyline::HostLimits::new());
    let mut kept = Vec::new();
    while let Some(page) = crawl.next().await {
        kept.push(page.unwrap().url().path().to_owned());
    }
    assert_eq!(kept, ["/start"]);
}

#[tokio::test]
async fn a_crawl_sends_credentials_only_to_its_seed_origin() {
    let other = TestServer::http(|_| html("<p>elsewhere</p>"))
        .await
        .unwrap();
    let location = format!("http://localhost:{}/landing", other.addr().port());
    let seed = TestServer::http(move |request| match request.target.as_str() {
        "/" => TestResponse::new(301).header("location", location.as_str()),
        _ => TestResponse::new(404),
    })
    .await
    .unwrap();
    let session = Session::builder()
        .bearer_auth("secret-token")
        .build()
        .unwrap();
    let mut crawl = session
        .crawl(seed.url("/"))
        .host_limits(leyline::HostLimits::new());
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    let asked = seed.requests().await;
    let first = asked.iter().find(|request| request.target == "/").unwrap();
    assert_eq!(first.header("authorization"), Some("Bearer secret-token"));
    let sent = other.requests().await;
    assert!(sent.iter().any(|request| request.target == "/landing"));
    assert!(
        sent.iter()
            .all(|request| request.header("authorization").is_none())
    );
}

#[tokio::test]
async fn a_redirected_seed_stops_at_its_first_off_site_page() {
    let third = TestServer::http(|_| html("<p>third</p>")).await.unwrap();
    let onward = format!("http://localhost:{}/third", third.addr().port());
    let second = TestServer::http(move |request| match request.target.as_str() {
        "/second" => TestResponse::new(302).header("location", onward.as_str()),
        _ => TestResponse::new(404),
    })
    .await
    .unwrap();
    let away = format!("http://localhost:{}/second", second.addr().port());
    let seed = TestServer::http(move |request| match request.target.as_str() {
        "/" => TestResponse::new(301).header("location", away.as_str()),
        _ => TestResponse::new(404),
    })
    .await
    .unwrap();
    let mut crawl = Session::new()
        .crawl(seed.url("/"))
        .host_limits(leyline::HostLimits::new());
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    assert!(targets(&second).await.contains("/second"));
    assert!(targets(&third).await.is_empty());
}

#[tokio::test]
async fn a_prefix_crawl_keeps_a_dotted_directory_as_its_prefix() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/v2.0" => html(r#"<a href="/v2.0/page">P</a><a href="/other">O</a>"#),
        _ => html("<p>page</p>"),
    })
    .await
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/v2.0"))
        .scope(Scope::Prefix)
        .host_limits(leyline::HostLimits::new());
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    let sent = page_targets(&server).await;
    assert!(sent.contains("/v2.0/page"));
    assert!(!sent.contains("/other"));
}

#[tokio::test]
async fn a_prefix_crawl_keeps_a_named_dotted_directory_as_its_prefix() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/docs.old" => html(r#"<a href="/docs.old/page">P</a><a href="/other">O</a>"#),
        _ => html("<p>page</p>"),
    })
    .await
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/docs.old"))
        .scope(Scope::Prefix)
        .host_limits(leyline::HostLimits::new());
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    let sent = page_targets(&server).await;
    assert!(sent.contains("/docs.old/page"));
    assert!(!sent.contains("/other"));
}

#[tokio::test]
async fn a_crawl_counts_a_redirect_that_leaves_its_scope() {
    let other = TestServer::http(|_| html("<p>elsewhere</p>"))
        .await
        .unwrap();
    let away = format!("http://localhost:{}/x", other.addr().port());
    let server = TestServer::http(move |request| match request.target.as_str() {
        "/" => html(r#"<a href="/out">O</a>"#),
        "/out" => TestResponse::new(302).header("location", away.as_str()),
        _ => TestResponse::new(404),
    })
    .await
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .host_limits(leyline::HostLimits::new());
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    assert_eq!(crawl.stats().skipped(SkipReason::Scope), 1);
}
