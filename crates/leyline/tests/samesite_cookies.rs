use leyline::testing::{TestResponse, TestServer, queue};
use leyline::{Browser, DnsConfig, Family, Preset, ProtocolPolicy, Session};

#[tokio::test]
async fn cross_site_initiator_withholds_samesite_cookies_by_destination() {
    let server = TestServer::http(queue(vec![
        TestResponse::new(200).body("ok").close(),
        TestResponse::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();
    let session = Session::builder()
        .browser(Browser::default())
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    let site = url::Url::parse(&server.url("/")).unwrap();
    session
        .cookies()
        .store_set_cookie("strict=1; SameSite=Strict", &site);
    session
        .cookies()
        .store_set_cookie("lax=1; SameSite=Lax", &site);

    for (path, preset) in [("/nav", Preset::FormNavigate), ("/xhr", Preset::Xhr)] {
        session
            .request(http::Method::GET, server.url(path))
            .preset(preset)
            .header("referer", "https://other.test/")
            .send()
            .await
            .unwrap();
    }

    let navigation = server.next_request().await.unwrap();
    let subresource = server.next_request().await.unwrap();
    assert_eq!(
        navigation.header_values("cookie"),
        vec!["lax=1"],
        "{}",
        navigation.text()
    );
    assert_eq!(
        subresource.header_count("cookie"),
        0,
        "{}",
        subresource.text()
    );
    server.shutdown().await;
}

#[tokio::test]
async fn samesite_follows_the_sent_fetch_site() {
    let server = TestServer::http(queue(vec![
        TestResponse::new(200).body("ok").close(),
        TestResponse::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();
    let site = url::Url::parse(&server.url("/")).unwrap();
    let preset_session = Session::builder()
        .browser(Browser::default())
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    let referer_session = Session::builder()
        .browser(Browser::default())
        .protocol(ProtocolPolicy::Http1)
        .headers([("referer", "https://other.test/")])
        .build()
        .unwrap();

    for (session, preset) in [
        (&preset_session, Preset::CrossOrigin),
        (&referer_session, Preset::Xhr),
    ] {
        session
            .cookies()
            .store_set_cookie("strict=1; SameSite=Strict", &site);
        session
            .cookies()
            .store_set_cookie("lax=1; SameSite=Lax", &site);
        session
            .get(server.url("/"))
            .preset(preset)
            .send()
            .await
            .unwrap();
        let req = server.next_request().await.unwrap();
        assert_eq!(req.header_values("sec-fetch-site"), vec!["cross-site"]);
        assert_eq!(req.header_count("cookie"), 0, "{}", req.text());
    }
    server.shutdown().await;
}

#[tokio::test]
async fn the_last_session_referer_sets_the_fetch_site() {
    let server = TestServer::http(queue(vec![TestResponse::new(200).body("ok").close()]))
        .await
        .unwrap();
    let site = url::Url::parse(&server.url("/")).unwrap();
    let session = Session::builder()
        .browser(Browser::default())
        .protocol(ProtocolPolicy::Http1)
        .headers([
            ("referer", "https://other.test/"),
            ("referer", site.as_str()),
        ])
        .build()
        .unwrap();
    session
        .cookies()
        .store_set_cookie("strict=1; SameSite=Strict", &site);

    session
        .get(server.url("/"))
        .preset(Preset::Xhr)
        .send()
        .await
        .unwrap();

    let req = server.next_request().await.unwrap();
    assert_eq!(req.header_values("sec-fetch-site"), vec!["same-origin"]);
    assert_eq!(
        req.header_values("cookie"),
        vec!["strict=1"],
        "{}",
        req.text()
    );
    server.shutdown().await;
}

fn port_of(server: &TestServer) -> u16 {
    url::Url::parse(&server.url("/")).unwrap().port().unwrap()
}

fn two_sites() -> DnsConfig {
    let local = "127.0.0.1:0".parse().unwrap();
    DnsConfig::new()
        .resolve_host("site-a.test", [local])
        .resolve_host("site-b.test", [local])
}

#[tokio::test]
async fn a_cross_site_redirect_chain_withholds_strict_cookies_for_firefox_only() {
    let landing = TestServer::http(queue(vec![
        TestResponse::new(200).body("ok").close(),
        TestResponse::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();
    let land = format!("http://site-a.test:{}/land", port_of(&landing));
    let start = TestServer::http(queue(vec![
        TestResponse::new(302)
            .close()
            .header("location", land.clone()),
        TestResponse::new(302)
            .close()
            .header("location", land.clone()),
    ]))
    .await
    .unwrap();
    let begin = format!("http://site-b.test:{}/start", port_of(&start));
    let site_a = url::Url::parse(&land).unwrap();

    for browser in [Browser::latest(Family::Firefox), Browser::default()] {
        let session = Session::builder()
            .browser(browser)
            .protocol(ProtocolPolicy::Http1)
            .dns(two_sites())
            .build()
            .unwrap();
        session
            .cookies()
            .store_set_cookie("strict=1; SameSite=Strict", &site_a);
        session
            .request(http::Method::GET, &begin)
            .preset(Preset::Navigate)
            .send()
            .await
            .unwrap();
        start.next_request().await.unwrap();
    }

    let firefox = landing.next_request().await.unwrap();
    let chrome = landing.next_request().await.unwrap();
    assert_eq!(firefox.header_count("cookie"), 0, "{}", firefox.text());
    assert_eq!(
        chrome.header_values("cookie"),
        vec!["strict=1"],
        "{}",
        chrome.text()
    );
    landing.shutdown().await;
    start.shutdown().await;
}

#[tokio::test]
async fn an_unusable_or_suppressed_session_referer_is_not_the_initiator() {
    let server = TestServer::http(queue(vec![
        TestResponse::new(200).body("ok").close(),
        TestResponse::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();
    let site = url::Url::parse(&server.url("/")).unwrap();
    let relative = Session::builder()
        .browser(Browser::default())
        .protocol(ProtocolPolicy::Http1)
        .headers([("referer", "/home")])
        .build()
        .unwrap();
    let suppressed = Session::builder()
        .browser(Browser::default())
        .protocol(ProtocolPolicy::Http1)
        .headers([("referer", "https://other.test/")])
        .build()
        .unwrap();

    relative
        .get(server.url("/"))
        .preset(Preset::Xhr)
        .send()
        .await
        .unwrap();
    suppressed
        .cookies()
        .store_set_cookie("strict=1; SameSite=Strict", &site);
    suppressed
        .get(server.url("/"))
        .preset(Preset::Xhr)
        .header("referer", "")
        .send()
        .await
        .unwrap();

    let first = server.next_request().await.unwrap();
    let second = server.next_request().await.unwrap();
    assert_eq!(first.header_values("sec-fetch-site"), vec!["same-origin"]);
    assert_eq!(second.header_values("sec-fetch-site"), vec!["same-origin"]);
    assert_eq!(
        second.header_values("cookie"),
        vec!["strict=1"],
        "{}",
        second.text()
    );
    server.shutdown().await;
}

#[tokio::test]
async fn origin_names_the_initiator() {
    let server = TestServer::http(queue(vec![TestResponse::new(200).body("ok").close()]))
        .await
        .unwrap();
    let session = Session::builder()
        .browser(Browser::default())
        .protocol(ProtocolPolicy::Http1)
        .headers([("referer", "https://other.test/page")])
        .build()
        .unwrap();

    session
        .get(server.url("/"))
        .preset(Preset::Xhr)
        .send()
        .await
        .unwrap();

    let req = server.next_request().await.unwrap();
    assert_eq!(req.header_values("sec-fetch-site"), vec!["cross-site"]);
    assert_eq!(
        req.header_values("origin"),
        vec!["https://other.test"],
        "{}",
        req.text()
    );
    server.shutdown().await;
}

#[tokio::test]
async fn chrome_judges_the_final_target_after_a_cross_site_bounce() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = TestServer::http_on(
        listener,
        queue(vec![
            TestResponse::new(302)
                .close()
                .header("location", format!("http://site-b.test:{port}/hop")),
            TestResponse::new(302)
                .close()
                .header("location", format!("http://site-a.test:{port}/end")),
            TestResponse::new(200).body("ok").close(),
        ]),
    )
    .unwrap();
    let site_a = url::Url::parse(&format!("http://site-a.test:{port}/")).unwrap();
    let session = Session::builder()
        .browser(Browser::default())
        .protocol(ProtocolPolicy::Http1)
        .dns(two_sites())
        .build()
        .unwrap();
    session
        .cookies()
        .store_set_cookie("strict=1; SameSite=Strict", &site_a);

    session
        .get(format!("http://site-a.test:{port}/start"))
        .preset(Preset::Xhr)
        .header("referer", site_a.as_str())
        .send()
        .await
        .unwrap();

    server.next_request().await.unwrap();
    server.next_request().await.unwrap();
    let end = server.next_request().await.unwrap();
    assert_eq!(
        end.header_values("cookie"),
        vec!["strict=1"],
        "{}",
        end.text()
    );
    server.shutdown().await;
}

#[tokio::test]
async fn a_relative_request_referer_is_same_origin() {
    let server = TestServer::http(queue(vec![TestResponse::new(200).body("ok").close()]))
        .await
        .unwrap();
    let site = url::Url::parse(&server.url("/")).unwrap();
    let session = Session::builder()
        .browser(Browser::default())
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    session
        .cookies()
        .store_set_cookie("strict=1; SameSite=Strict", &site);

    session
        .get(server.url("/x"))
        .preset(Preset::Xhr)
        .header("referer", "/home")
        .send()
        .await
        .unwrap();

    let req = server.next_request().await.unwrap();
    assert_eq!(req.header_values("sec-fetch-site"), vec!["same-origin"]);
    assert_eq!(
        req.header_values("cookie"),
        vec!["strict=1"],
        "{}",
        req.text()
    );
    server.shutdown().await;
}

#[tokio::test]
async fn an_https_initiator_sends_a_null_origin_to_http() {
    let server = TestServer::http(queue(vec![TestResponse::new(200).body("ok").close()]))
        .await
        .unwrap();
    let session = Session::builder()
        .browser(Browser::default())
        .protocol(ProtocolPolicy::Http1)
        .headers([("referer", "https://other.test/page")])
        .build()
        .unwrap();

    session
        .post(server.url("/form"))
        .preset(Preset::FormNavigate)
        .body("a=1")
        .send()
        .await
        .unwrap();

    let req = server.next_request().await.unwrap();
    assert_eq!(req.header_values("origin"), vec!["null"], "{}", req.text());
    server.shutdown().await;
}
