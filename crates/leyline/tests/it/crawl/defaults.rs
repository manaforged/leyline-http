use super::*;

#[tokio::test]
async fn a_crawl_on_a_plain_session_waits_its_turn_per_host() {
    let stamps = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = stamps.clone();
    let server = TestServer::http(move |request| {
        seen.lock().unwrap().push(std::time::Instant::now());
        match request.target.as_str() {
            "/" => html(r#"<a href="/a">A</a><a href="/b">B</a>"#),
            path => html(&format!("<p>{path}</p>")),
        }
    })
    .await
    .unwrap();
    let mut crawl = Session::new().crawl(server.url("/"));
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    let stamps = stamps.lock().unwrap().clone();
    assert_eq!(stamps.len(), 5);
    for pair in stamps.windows(2) {
        assert!(
            pair[1] - pair[0] >= Duration::from_millis(900),
            "{:?}",
            pair[1] - pair[0]
        );
    }
}

#[tokio::test]
async fn a_crawl_requests_one_url_per_normalized_address() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/" => html(
            r#"<a href="/p?b=2&a=1">1</a><a href="/p?a=1&b=2">2</a><a href="/%7Euser/">3</a><a href="/~user/">4</a>"#,
        ),
        path => html(&format!("<p>{path}</p>")),
    })
    .await
    .unwrap();
    let session = Session::builder()
        .host_limits(leyline::HostLimits::new().max_in_flight(4))
        .build()
        .unwrap();
    let mut crawl = session.crawl(server.url("/"));
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    let sent: Vec<String> = server
        .requests()
        .await
        .into_iter()
        .map(|request| request.target)
        .filter(|target| target != "/robots.txt" && target != "/sitemap.xml")
        .collect();
    assert_eq!(sent.len(), 3, "{sent:?}");
    assert!(sent.contains(&"/p?b=2&a=1".to_owned()), "{sent:?}");
}

#[tokio::test]
async fn a_crawl_takes_its_own_host_limits_over_the_polite_default() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/" => html(r#"<a href="/a">A</a><a href="/b">B</a>"#),
        path => html(&format!("<p>{path}</p>")),
    })
    .await
    .unwrap();
    let started = std::time::Instant::now();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .host_limits(leyline::HostLimits::new());
    let mut pages = 0;
    while let Some(page) = crawl.next().await {
        page.unwrap();
        pages += 1;
    }
    assert_eq!(pages, 3);
    assert!(
        started.elapsed() < Duration::from_millis(800),
        "{:?}",
        started.elapsed()
    );
}
