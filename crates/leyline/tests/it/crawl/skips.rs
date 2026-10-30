use super::*;

async fn kept_and_stats(
    server: &testing::Server,
    respect: Respect,
) -> (BTreeSet<String>, leyline::Crawl) {
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .respect(respect)
        .host_limits(leyline::HostLimits::new());
    let mut kept = BTreeSet::new();
    while let Some(page) = crawl.next().await {
        kept.insert(page.unwrap().url().path().to_owned());
    }
    (kept, crawl)
}

#[tokio::test]
async fn a_crawl_skips_a_block_page() {
    let server = testing::Server::http(|request| match request.target.as_str() {
        "/" => html(r#"<a href="/wall">W</a>"#),
        "/wall" => html("<p>checking your browser</p>").header("cf-mitigated", "challenge"),
        _ => testing::Response::new(404),
    })
    .await
    .unwrap();
    let (kept, stats) = kept_and_stats(&server, Respect::Robots).await;
    assert_eq!(kept, BTreeSet::from(["/".to_owned()]));
    assert_eq!(stats.skipped(SkipReason::Blocked), 1);
}

#[tokio::test]
async fn a_training_crawl_skips_a_noai_page_and_a_robots_crawl_keeps_it() {
    let server = testing::Server::http(|request| match request.target.as_str() {
        "/" => html(r#"<a href="/art">A</a>"#),
        "/art" => html(r#"<meta name="robots" content="noai"><p>artwork</p>"#),
        _ => testing::Response::new(404),
    })
    .await
    .unwrap();
    let (kept, stats) = kept_and_stats(&server, Respect::AiTraining).await;
    assert_eq!(kept, BTreeSet::from(["/".to_owned()]));
    assert_eq!(stats.skipped(SkipReason::NoAi), 1);
    let (kept, _) = kept_and_stats(&server, Respect::Robots).await;
    assert!(kept.contains("/art"));
}

#[tokio::test]
async fn a_crawl_gives_up_after_five_redirects() {
    let server = testing::Server::http(|request| {
        let target = request.target.as_str();
        match target.strip_prefix("/hop") {
            Some(n) => {
                let next = n.parse::<u32>().unwrap() + 1;
                testing::Response::new(302).header("location", format!("/hop{next}"))
            }
            None if target == "/" => html(r#"<a href="/hop0">H</a>"#),
            None => testing::Response::new(404),
        }
    })
    .await
    .unwrap();
    let (kept, stats) = kept_and_stats(&server, Respect::Robots).await;
    assert_eq!(kept, BTreeSet::from(["/".to_owned()]));
    assert_eq!(stats.skipped(SkipReason::Redirects), 1);
}

#[tokio::test]
async fn a_crawl_obeys_a_nofollow_page_and_an_x_robots_tag() {
    let server = testing::Server::http(|request| match request.target.as_str() {
        "/" => html(r#"<a href="/closed">C</a><a href="/tagged">T</a>"#),
        "/closed" => html(r#"<meta name="robots" content="nofollow"><a href="/behind">B</a>"#),
        "/tagged" => html("<p>tagged</p>").header("x-robots-tag", "noindex"),
        _ => html("<p>behind</p>"),
    })
    .await
    .unwrap();
    let (kept, stats) = kept_and_stats(&server, Respect::Robots).await;
    assert!(!targets(&server).await.contains("/behind"));
    assert!(!kept.contains("/tagged"));
    assert_eq!(stats.skipped(SkipReason::NoIndex), 1);
}

#[tokio::test]
async fn a_crawl_skips_a_site_whose_robots_txt_stays_down() {
    let server = testing::Server::http(|request| match request.target.as_str() {
        "/robots.txt" => testing::Response::new(503),
        _ => html("<p>page</p>"),
    })
    .await
    .unwrap();
    let (kept, stats) = kept_and_stats(&server, Respect::Robots).await;
    let robots = server
        .requests()
        .await
        .iter()
        .filter(|request| request.target == "/robots.txt")
        .count();
    assert_eq!(robots, 5);
    assert!(kept.is_empty());
    assert!(page_targets(&server).await.is_empty());
    assert_eq!(stats.skipped(SkipReason::Robots), 1);
}

#[tokio::test]
async fn a_nofollow_page_does_not_queue_its_canonical_page() {
    let server = testing::Server::http(|request| match request.target.as_str() {
        "/" => html(r#"<a href="/alias">A</a>"#),
        "/alias" => html(
            r#"<meta name="robots" content="nofollow"><link rel="canonical" href="/target"><p>alias</p>"#,
        ),
        _ => html("<p>target</p>"),
    })
    .await
    .unwrap();
    let (kept, _) = kept_and_stats(&server, Respect::Robots).await;
    assert!(kept.contains("/alias"));
    assert!(!targets(&server).await.contains("/target"));
}

#[tokio::test]
async fn a_crawl_page_hides_url_secrets_in_debug_output() {
    let server = testing::Server::http(|_| html("<p>home</p>"))
        .await
        .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/?token=hunter2"))
        .host_limits(leyline::HostLimits::new());
    let page = crawl.next().await.unwrap().unwrap();
    let shown = format!("{page:?}");
    assert!(!shown.contains("hunter2"), "{shown}");
}

#[tokio::test]
async fn a_stalled_page_body_ends_at_the_session_deadline() {
    let server = testing::Server::http(|request| match request.target.as_str() {
        "/robots.txt" => testing::Response::new(404),
        "/" => html(r#"<a href="/stall">S</a>"#),
        _ => testing::Response::new(200)
            .header("content-type", "text/html")
            .chunks(["<p>start", " end</p>"], Duration::from_secs(30)),
    })
    .await
    .unwrap();
    let session = Session::builder()
        .timeout(leyline::Timeout::new().total(Duration::from_secs(2)))
        .build()
        .unwrap();
    let mut crawl = session
        .crawl(server.url("/"))
        .host_limits(leyline::HostLimits::new());
    let ended = tokio::time::timeout(Duration::from_secs(10), async {
        let mut errors = 0;
        while let Some(page) = crawl.next().await {
            errors += usize::from(page.is_err());
        }
        errors
    })
    .await;
    assert_eq!(ended, Ok(1));
}
