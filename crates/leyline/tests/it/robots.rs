use std::time::Duration;

use leyline::testing::{TestResponse, TestServer};
use leyline::{Purpose, Robots, RobotsSource, Session, Url};

const RFC_EXAMPLE: &str = "User-Agent: *
Disallow: *.gif$
Disallow: /example/
Allow: /publications/

User-Agent: foobot
Disallow:/
Allow:/example/page.html
Allow:/example/allowed.gif

User-Agent: barbot
User-Agent: bazbot
Disallow: /example/page.html

User-Agent: quxbot
";

fn at(path: &str) -> Url {
    Url::parse(&format!("https://example.com{path}")).unwrap()
}

#[test]
fn robots_follow_the_rfc_example() {
    for agent in ["*", "OtherBot"] {
        let robots = Robots::parse(RFC_EXAMPLE, agent);
        assert!(!robots.allows(&at("/image.gif")), "{agent}");
        assert!(!robots.allows(&at("/a/b/c.gif")), "{agent}");
        assert!(robots.allows(&at("/image.gif?size=2")), "{agent}");
        assert!(!robots.allows(&at("/example/page.html")), "{agent}");
        assert!(robots.allows(&at("/publications/paper.html")), "{agent}");
        assert!(robots.allows(&at("/robots.txt")), "{agent}");
    }
    let foobot = Robots::parse(RFC_EXAMPLE, "FooBot");
    assert!(!foobot.allows(&at("/")));
    assert!(!foobot.allows(&at("/publications/")));
    assert!(foobot.allows(&at("/example/page.html")));
    assert!(foobot.allows(&at("/example/allowed.gif")));
    assert!(foobot.allows(&at("/robots.txt")));
    for agent in ["barbot", "bazbot"] {
        let robots = Robots::parse(RFC_EXAMPLE, agent);
        assert!(!robots.allows(&at("/example/page.html")), "{agent}");
        assert!(robots.allows(&at("/example/other.html")), "{agent}");
        assert!(robots.allows(&at("/image.gif")), "{agent}");
    }
    assert!(Robots::parse(RFC_EXAMPLE, "quxbot").allows(&at("/example/page.html")));
}

#[test]
fn robots_use_the_longest_match_and_allow_wins_a_tie() {
    let robots = Robots::parse(
        "User-Agent: foobot\nAllow: /example/page/\nDisallow: /example/page/disallowed.gif\nAllow: /tie\nDisallow: /tie\n",
        "foobot",
    );
    assert!(!robots.allows(&at("/example/page/disallowed.gif")));
    assert!(robots.allows(&at("/example/page/other.gif")));
    assert!(robots.allows(&at("/tie")));
}

#[test]
fn robots_merge_groups_and_ignore_rules_outside_groups() {
    let text = "Disallow: /before\nuser-agent: ExampleBot\ndisallow: /foo\ndisallow: /bar\n\nuser-agent: BazBot\ndisallow: /qux\n\nuser-agent: ExampleBot\nsitemap: https://example.com/a.xml\ndisallow: /baz\n";
    let robots = Robots::parse(text, "examplebot");
    for path in ["/foo", "/bar", "/baz"] {
        assert!(!robots.allows(&at(path)), "{path}");
    }
    assert!(robots.allows(&at("/qux")));
    assert!(robots.allows(&at("/before")));
    assert!(Robots::parse(text, "OtherBot").allows(&at("/foo")));
}

#[test]
fn robots_compare_percent_encoded_paths() {
    let robots = Robots::parse(
        "User-agent: *\nDisallow: /foo/bar/%62%61%7A\nDisallow: /tsu/ツ\nDisallow: /path/file-with-a-%2A.html\nDisallow: /exact$\nDisallow: /a/*/c\n",
        "*",
    );
    assert!(!robots.allows(&at("/foo/bar/baz")));
    assert!(!robots.allows(&at("/tsu/%E3%83%84")));
    assert!(!robots.allows(&at("/path/file-with-a-*.html")));
    assert!(robots.allows(&at("/path/file-with-a-x.html")));
    assert!(!robots.allows(&at("/exact")));
    assert!(robots.allows(&at("/exact/more")));
    assert!(!robots.allows(&at("/a/b/c")));
    assert!(!robots.allows(&at("/a/b/b/c/d")));
    assert!(robots.allows(&at("/a/c")));
}

#[test]
fn robots_read_signals_delay_and_sitemaps() {
    let robots = Robots::parse(
        "Sitemap: https://example.com/one.xml\nUser-agent: *\nContent-Signal: search=yes, ai-train=no, use=reference\nCrawl-delay: 2.5\nDisallow: /private/\nSitemap: https://example.com/two.xml\n",
        "*",
    );
    assert!(!robots.allows_use(&at("/page"), Purpose::AiTrain));
    assert!(robots.allows_use(&at("/page"), Purpose::Search));
    assert!(robots.allows_use(&at("/page"), Purpose::AiInput));
    assert_eq!(robots.crawl_delay(), Some(Duration::from_millis(2500)));
    assert_eq!(
        robots.sitemaps(),
        ["https://example.com/one.xml", "https://example.com/two.xml"]
    );
    assert!(!robots.allows(&at("/private/x")));
    assert_eq!(robots.source(), RobotsSource::Parsed);
}

#[tokio::test]
async fn session_robots_applies_the_status_rules() {
    let server = TestServer::http(|request| match request.target.as_str() {
        "/robots.txt" => TestResponse::new(301).header("location", "/moved.txt"),
        "/moved.txt" => TestResponse::new(200).body("User-agent: *\nDisallow: /secret\n"),
        _ => TestResponse::new(404),
    })
    .await
    .unwrap();
    let session = Session::new();
    let robots = session
        .robots(server.url("/deep/page.html"), "*")
        .await
        .unwrap();
    assert!(!robots.allows(&Url::parse(&server.url("/secret")).unwrap()));
    assert!(robots.allows(&Url::parse(&server.url("/open")).unwrap()));
    let targets: Vec<String> = server
        .requests()
        .await
        .into_iter()
        .map(|request| request.target)
        .collect();
    assert_eq!(targets, ["/robots.txt", "/moved.txt"]);

    let missing = TestServer::http(|_| TestResponse::new(404)).await.unwrap();
    let robots = session.robots(missing.url("/"), "*").await.unwrap();
    assert_eq!(robots.source(), RobotsSource::Unavailable);
    assert!(robots.allows(&Url::parse(&missing.url("/anything")).unwrap()));

    let broken = TestServer::http(|_| TestResponse::new(503)).await.unwrap();
    let robots = session.robots(broken.url("/"), "*").await.unwrap();
    assert_eq!(robots.source(), RobotsSource::Unreachable);
    assert!(!robots.allows(&Url::parse(&broken.url("/anything")).unwrap()));
    assert!(robots.allows(&Url::parse(&broken.url("/robots.txt")).unwrap()));

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let closed = format!("http://{}/", listener.local_addr().unwrap());
    drop(listener);
    let robots = session.robots(closed.as_str(), "*").await.unwrap();
    assert_eq!(robots.source(), RobotsSource::Unreachable);
}

#[tokio::test]
async fn session_robots_reads_at_least_500_kib() {
    let mut body = "#".repeat(450 * 1024);
    body.push_str("\nUser-agent: *\nDisallow: /late\n");
    let server = TestServer::http(move |_| TestResponse::new(200).body(body.clone()))
        .await
        .unwrap();
    let robots = Session::new().robots(server.url("/"), "*").await.unwrap();
    assert!(!robots.allows(&Url::parse(&server.url("/late")).unwrap()));
}

#[test]
fn robots_read_content_usage_by_the_longest_matching_path() {
    let robots = Robots::parse(
        "User-agent: *\nContent-Usage: train-ai=n, search=y, unknown-label=y\nContent-Usage: /ai-ok/ train-ai=y\nContent-Usage: /no-search/ search=n\nContent-Signal: ai-input=yes\n",
        "*",
    );
    assert!(!robots.allows_use(&at("/page"), Purpose::AiTrain));
    assert!(robots.allows_use(&at("/ai-ok/page"), Purpose::AiTrain));
    assert!(robots.allows_use(&at("/page"), Purpose::Search));
    assert!(!robots.allows_use(&at("/no-search/page"), Purpose::Search));
    assert!(robots.allows_use(&at("/page"), Purpose::AiInput));

    let open = Robots::parse("User-agent: *\nContent-Usage: /ai-ok/ train-ai=y\n", "*");
    assert!(open.allows_use(&at("/ai-ok/page"), Purpose::AiTrain));
    assert!(open.allows_use(&at("/other"), Purpose::AiTrain));
}

#[test]
fn robots_survive_a_huge_crawl_delay() {
    let robots = Robots::parse("User-agent: *\nCrawl-delay: 1e300\n", "*");
    assert!(
        robots
            .crawl_delay()
            .is_some_and(|gap| gap > Duration::from_secs(3600))
    );
}

#[test]
fn robots_start_a_new_group_after_any_group_record() {
    let robots = Robots::parse(
        "User-agent: a\nCrawl-delay: 5\nUser-agent: *\nDisallow: /p\n",
        "*",
    );
    assert_eq!(robots.crawl_delay(), None);
    assert!(!robots.allows(&at("/p")));
    let robots = Robots::parse(
        "User-agent: *\nCrawl-delay: 10\n\nUser-agent: MJ12bot\nDisallow: /\n",
        "*",
    );
    assert_eq!(robots.crawl_delay(), Some(Duration::from_secs(10)));
    assert!(robots.allows(&at("/page")));
}

#[test]
fn robots_match_a_dollar_inside_a_pattern_literally() {
    let robots = Robots::parse("User-agent: *\nDisallow: /price$list\n", "*");
    assert!(!robots.allows(&at("/price$list")));
    assert!(robots.allows(&at("/price")));
}

#[test]
fn robots_reject_a_sign_inside_a_percent_escape() {
    let robots = Robots::parse("User-agent: *\nDisallow: /a%+41\n", "*");
    assert!(robots.allows(&at("/a%041")));
    assert!(!robots.allows(&at("/a%+41")));
}

#[test]
fn robots_read_usage_values_with_parameters() {
    let robots = Robots::parse("User-agent: *\nContent-Usage: train-ai=n;src=site\n", "*");
    assert!(!robots.allows_use(&at("/page"), Purpose::AiTrain));
}

#[test]
fn robots_match_the_product_token_and_only_a_star_means_everyone() {
    let text = "User-agent: Googlebot/2.1\nDisallow: /g\n\nUser-agent: 123\nDisallow: /\n\nUser-agent: *\nDisallow: /all\n";
    let google = Robots::parse(text, "Googlebot/2.1 (+http://www.google.com/bot.html)");
    assert!(!google.allows(&at("/g")));
    assert!(google.allows(&at("/all")));
    let anyone = Robots::parse(text, "*");
    assert!(anyone.allows(&at("/g")));
    assert!(!anyone.allows(&at("/all")));
}

#[tokio::test]
async fn session_robots_treats_rate_limiting_as_unreachable() {
    let server = TestServer::http(|_| TestResponse::new(429).header("retry-after", "0"))
        .await
        .unwrap();
    let robots = Session::new().robots(server.url("/"), "*").await.unwrap();
    assert_eq!(robots.source(), RobotsSource::Unreachable);
    assert!(!robots.allows(&at("/page")));
}

#[tokio::test]
async fn session_robots_drops_the_line_cut_at_the_size_limit() {
    let head = "User-agent: *\nDisallow: /p\n#";
    let tail = "\nAllow: /private-ok\n";
    let cut = 512 * 1024;
    let filler = "x".repeat(cut - head.len() - "\nAllow: /p".len());
    let body = format!("{head}{filler}{tail}");
    let server = TestServer::http(move |request| match request.target.as_str() {
        "/robots.txt" => TestResponse::new(200).body(body.clone()),
        _ => TestResponse::new(404),
    })
    .await
    .unwrap();
    let robots = Session::new().robots(server.url("/"), "*").await.unwrap();
    assert!(!robots.allows(&at("/private-x")));
}
