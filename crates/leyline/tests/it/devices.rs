use std::time::Duration;

use leyline::cookie::Jar;
use leyline::testing::{TestResponse, TestServer};
use leyline::{Browser, ChromiumBrand, Device, Kind, Platform, ProxyUrl, Session};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("leyline-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn url() -> url::Url {
    url::Url::parse("https://shop.example/").unwrap()
}

#[tokio::test]
async fn a_jar_saves_loads_and_autosaves() {
    let dir = scratch("jar");
    let path = dir.join("jar.json");
    let jar = Jar::new();
    jar.store_set_cookie("session-token=a; Path=/; Max-Age=3600", &url());
    jar.save_to(&path).unwrap();
    let loaded = Jar::load_from(&path).unwrap();
    assert_eq!(
        loaded.get_cookie(&url(), "session-token").as_deref(),
        Some("a")
    );

    let autosave = loaded.autosave(&path, Duration::from_millis(20));
    loaded.store_set_cookie("session-token=b; Path=/; Max-Age=3600", &url());
    autosave.flush().await.unwrap();
    let flushed = Jar::load_from(&path).unwrap();
    assert_eq!(
        flushed.get_cookie(&url(), "session-token").as_deref(),
        Some("b")
    );

    loaded.store_set_cookie("session-token=c; Path=/; Max-Age=3600", &url());
    autosave.shutdown().await.unwrap();
    let last = Jar::load_from(&path).unwrap();
    assert_eq!(
        last.get_cookie(&url(), "session-token").as_deref(),
        Some("c")
    );
    drop(std::fs::remove_dir_all(&dir));
}

#[test]
fn a_proxy_url_round_trips_with_its_credentials() {
    let proxy = ProxyUrl::parse("http://user:secret@proxy.example:8080").unwrap();
    let json = serde_json::to_string(&proxy).unwrap();
    assert!(json.contains("secret"));
    let back: ProxyUrl = serde_json::from_str(&json).unwrap();
    assert_eq!(back, proxy);
    assert!(!format!("{proxy}").contains("secret"));
}

#[test]
fn a_device_reopens_as_the_same_browser_and_refuses_drift() {
    let dir = scratch("device");
    let session = Session::builder()
        .browser(Browser::Chrome154)
        .platform(Platform::MacOS)
        .brand(ChromiumBrand::Edge)
        .languages(["de-DE", "de"])
        .build()
        .unwrap();
    let proxy = ProxyUrl::parse("http://user:pw@proxy.example:8080").unwrap();
    let mut device = Device::capture(&session, Some(proxy.clone()));
    device.pin_profile(&session).unwrap();
    let path = dir.join("device.json");
    device.save_to(&path).unwrap();

    let restored = Device::load_from(&path).unwrap();
    assert_eq!(restored.proxy, Some(proxy));
    let reopened = restored.open().unwrap();
    assert_eq!(
        reopened.identity().profile_id(),
        session.identity().profile_id()
    );
    assert_eq!(reopened.identity().brand(), Some(ChromiumBrand::Edge));
    assert_eq!(reopened.identity().platform(), Platform::MacOS);
    restored.check(&reopened).unwrap();

    let mut drifted = restored.clone();
    drifted.profile_id = Some("0000000000000000".to_owned());
    assert_eq!(drifted.check(&reopened).unwrap_err().kind(), Kind::Config);
    drop(std::fs::remove_dir_all(&dir));
}

fn sts_server_reply(req: &leyline::testing::RecordedRequest) -> TestResponse {
    if req.target == "/secure" {
        TestResponse::new(200).header("strict-transport-security", "max-age=3600")
    } else {
        TestResponse::new(200).body(b"upgraded".to_vec())
    }
}

#[tokio::test]
async fn hsts_upgrades_later_http_requests_and_survives_a_restart() {
    let server = TestServer::https(sts_server_reply).await.unwrap();
    let port = server.addr().port();
    let session = Session::builder()
        .browser(Browser::default())
        .tls_trust(server.trust())
        .build()
        .unwrap();
    session
        .get(format!("https://localhost:{port}/secure"))
        .await
        .unwrap();
    let upgraded = session
        .get(format!("http://localhost:{port}/plain"))
        .await
        .unwrap();
    assert_eq!(upgraded.url().scheme(), "https");
    assert_eq!(upgraded.text().await.unwrap(), "upgraded");

    let state = session.state();
    let saved = serde_json::to_value(&state).unwrap();
    assert_eq!(saved["hsts"].as_array().map(Vec::len), Some(1));
    assert!(!saved["tls_sessions"].as_array().unwrap().is_empty());

    let fresh = Session::builder()
        .browser(Browser::default())
        .tls_trust(server.trust())
        .build()
        .unwrap();
    state.restore_into(&fresh);
    let again = fresh
        .get(format!("http://localhost:{port}/plain"))
        .await
        .unwrap();
    assert_eq!(again.url().scheme(), "https");
}

fn site(req: &leyline::testing::RecordedRequest) -> TestResponse {
    match req.target.as_str() {
        "/start" => TestResponse::new(302).header("location", "/page"),
        _ => TestResponse::new(200),
    }
}

#[tokio::test]
async fn a_tab_tracks_the_page_for_later_requests() {
    let server = TestServer::http(site).await.unwrap();
    let tab = Session::builder()
        .browser(Browser::default())
        .protocol(leyline::ProtocolPolicy::Http1)
        .build()
        .unwrap()
        .tab();
    tab.open(server.url("/start")).await.unwrap();
    assert_eq!(tab.current().map(String::from), Some(server.url("/page")));
    tab.xhr("/api").await.unwrap();
    tab.follow("/next").await.unwrap();
    assert_eq!(tab.current().map(String::from), Some(server.url("/next")));

    let seen = server.requests().await;
    let api = seen.iter().find(|r| r.target == "/api").unwrap();
    assert_eq!(api.header("referer"), Some(server.url("/page").as_str()));
    assert_eq!(api.header("sec-fetch-site"), Some("same-origin"));
    let next = seen.iter().find(|r| r.target == "/next").unwrap();
    assert_eq!(next.header("referer"), Some(server.url("/page").as_str()));
    assert_eq!(next.header("sec-fetch-site"), Some("same-origin"));
    let start = seen.iter().find(|r| r.target == "/start").unwrap();
    assert_eq!(start.header("sec-fetch-site"), Some("none"));
}

#[test]
fn redact_url_hides_secrets() {
    assert_eq!(
        leyline::redact_url("https://user:secret@example.com/a?token=1#frag"),
        "https://user:***@example.com/a?***"
    );
}
