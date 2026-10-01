#[path = "core_support/raw_server.rs"]
mod raw_server;

use leyline::{Browser, DnsConfig, Family, Preset, ProtocolPolicy, Session};
use raw_server::{RawResponse, RawServer};

#[tokio::test]
async fn cross_site_initiator_withholds_samesite_cookies_by_destination() {
    let mut server = RawServer::start(vec![RawResponse::ok(), RawResponse::ok()]).await;
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

    let navigation = server.next_request().await;
    let subresource = server.next_request().await;
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
    server.finish().await;
}

#[tokio::test]
async fn samesite_follows_the_sent_fetch_site() {
    let mut server = RawServer::start(vec![RawResponse::ok(), RawResponse::ok()]).await;
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
        let req = server.next_request().await;
        assert_eq!(req.header_values("sec-fetch-site"), vec!["cross-site"]);
        assert_eq!(req.header_count("cookie"), 0, "{}", req.text());
    }
    server.finish().await;
}

#[tokio::test]
async fn the_last_session_referer_sets_the_fetch_site() {
    let mut server = RawServer::start(vec![RawResponse::ok()]).await;
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

    let req = server.next_request().await;
    assert_eq!(req.header_values("sec-fetch-site"), vec!["same-origin"]);
    assert_eq!(
        req.header_values("cookie"),
        vec!["strict=1"],
        "{}",
        req.text()
    );
    server.finish().await;
}

fn port_of(server: &RawServer) -> u16 {
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
    let mut landing = RawServer::start(vec![RawResponse::ok(), RawResponse::ok()]).await;
    let land = format!("http://site-a.test:{}/land", port_of(&landing));
    let mut start = RawServer::start(vec![
        RawResponse::redirect(land.clone()),
        RawResponse::redirect(land.clone()),
    ])
    .await;
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
        start.next_request().await;
    }

    let firefox = landing.next_request().await;
    let chrome = landing.next_request().await;
    assert_eq!(firefox.header_count("cookie"), 0, "{}", firefox.text());
    assert_eq!(
        chrome.header_values("cookie"),
        vec!["strict=1"],
        "{}",
        chrome.text()
    );
    landing.finish().await;
    start.finish().await;
}

#[tokio::test]
async fn an_unusable_or_suppressed_session_referer_is_not_the_initiator() {
    let mut server = RawServer::start(vec![RawResponse::ok(), RawResponse::ok()]).await;
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

    let first = server.next_request().await;
    let second = server.next_request().await;
    assert_eq!(first.header_values("sec-fetch-site"), vec!["same-origin"]);
    assert_eq!(second.header_values("sec-fetch-site"), vec!["same-origin"]);
    assert_eq!(
        second.header_values("cookie"),
        vec!["strict=1"],
        "{}",
        second.text()
    );
    server.finish().await;
}

#[tokio::test]
async fn origin_names_the_initiator() {
    let mut server = RawServer::start(vec![RawResponse::ok()]).await;
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

    let req = server.next_request().await;
    assert_eq!(req.header_values("sec-fetch-site"), vec!["cross-site"]);
    assert_eq!(
        req.header_values("origin"),
        vec!["https://other.test"],
        "{}",
        req.text()
    );
    server.finish().await;
}

#[tokio::test]
async fn chrome_judges_the_final_target_after_a_cross_site_bounce() {
    let listener = RawServer::bind().await;
    let port = listener.local_addr().unwrap().port();
    let mut server = RawServer::serve(
        listener,
        vec![
            RawResponse::redirect(format!("http://site-b.test:{port}/hop")),
            RawResponse::redirect(format!("http://site-a.test:{port}/end")),
            RawResponse::ok(),
        ],
    );
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

    server.next_request().await;
    server.next_request().await;
    let end = server.next_request().await;
    assert_eq!(
        end.header_values("cookie"),
        vec!["strict=1"],
        "{}",
        end.text()
    );
    server.finish().await;
}

#[tokio::test]
async fn a_relative_request_referer_is_same_origin() {
    let mut server = RawServer::start(vec![RawResponse::ok()]).await;
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

    let req = server.next_request().await;
    assert_eq!(req.header_values("sec-fetch-site"), vec!["same-origin"]);
    assert_eq!(
        req.header_values("cookie"),
        vec!["strict=1"],
        "{}",
        req.text()
    );
    server.finish().await;
}

#[tokio::test]
async fn an_https_initiator_sends_a_null_origin_to_http() {
    let mut server = RawServer::start(vec![RawResponse::ok()]).await;
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

    let req = server.next_request().await;
    assert_eq!(req.header_values("origin"), vec!["null"], "{}", req.text());
    server.finish().await;
}
