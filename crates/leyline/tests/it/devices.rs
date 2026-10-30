use std::time::{Duration, Instant};

use leyline::testing;
use leyline::{Browser, Kind, Platform, Session};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("leyline-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn url() -> url::Url {
    url::Url::parse("https://shop.example/").unwrap()
}

#[tokio::test]
async fn a_session_saves_loads_and_autosaves_its_cookies() {
    let dir = scratch("jar");
    let path = dir.join("device.json");
    let token = |path: &std::path::Path| {
        Session::load(path)
            .unwrap()
            .build()
            .unwrap()
            .cookies()
            .get(&url(), "session-token")
    };
    let session = Session::new();
    session
        .cookies()
        .store("session-token=a; Path=/; Max-Age=3600", &url());
    session.save(&path).unwrap();
    assert_eq!(token(&path).as_deref(), Some("a"));

    let reopened = Session::load(&path).unwrap().build().unwrap();
    reopened.autosave(&path, Duration::from_secs(3600));
    reopened
        .cookies()
        .store("session-token=b; Path=/; Max-Age=3600", &url());
    reopened.shutdown();
    let deadline = Instant::now() + Duration::from_secs(5);
    while token(&path).as_deref() != Some("b") && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(token(&path).as_deref(), Some("b"));
    drop(std::fs::remove_dir_all(&dir));
}

#[test]
fn a_session_reopens_as_the_same_browser_and_refuses_drift() {
    let dir = scratch("device");
    let session = Session::builder()
        .browser(Browser::Edge154)
        .platform(Platform::MacOS)
        .languages(["de-DE", "de"])
        .proxy("http://user:pw@proxy.example:8080")
        .build()
        .unwrap();
    let path = dir.join("device.json");
    session.save(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains(":pw@"), "{text}");
    assert!(text.contains("proxy.example"), "{text}");

    let reopened = Session::load(&path).unwrap().build().unwrap();
    assert!(session.profile_id().is_some());
    assert_eq!(reopened.profile_id(), session.profile_id());
    assert!(
        reopened.user_agent().contains(" Edg/154."),
        "{}",
        reopened.user_agent()
    );
    assert_eq!(reopened.platform(), Platform::MacOS);
    assert_eq!(reopened.languages(), session.languages());
    let proxy = reopened.proxy();
    assert!(
        proxy.is_some_and(|proxy| proxy.contains("proxy.example")),
        "{proxy:?}"
    );

    let drifted = Session::load(&path)
        .unwrap()
        .browser(Browser::Chrome154)
        .platform(Platform::Windows)
        .build()
        .unwrap_err();
    assert_eq!(drifted.kind(), Kind::Config);
    assert!(drifted.is_profile_changed(), "{drifted}");
    drop(std::fs::remove_dir_all(&dir));
}

fn sts_server_reply(req: &leyline::testing::Request) -> testing::Response {
    if req.target == "/secure" {
        testing::Response::new(200).header("strict-transport-security", "max-age=3600")
    } else {
        testing::Response::new(200).body(b"upgraded".to_vec())
    }
}

#[tokio::test]
async fn hsts_upgrades_later_http_requests_and_survives_a_restart() {
    let dir = scratch("hsts");
    let path = dir.join("device.json");
    let server = testing::Server::https(sts_server_reply).await.unwrap();
    let port = server.addr().port();
    let session = Session::builder()
        .browser(Browser::default())
        .tls(server.tls())
        .build()
        .unwrap();
    session
        .get(format!("https://localhost:{port}/secure"))
        .send()
        .await
        .unwrap();
    let upgraded = session
        .get(format!("http://localhost:{port}/plain"))
        .send()
        .await
        .unwrap();
    assert_eq!(upgraded.url().scheme(), "https");
    assert_eq!(upgraded.text().await.unwrap(), "upgraded");

    session.save(&path).unwrap();
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["state"]["hsts"].as_array().map(Vec::len), Some(1));
    assert!(
        !saved["state"]["tls_sessions"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let fresh = Session::load(&path)
        .unwrap()
        .tls(server.tls())
        .build()
        .unwrap();
    let again = fresh
        .get(format!("http://localhost:{port}/plain"))
        .send()
        .await
        .unwrap();
    assert_eq!(again.url().scheme(), "https");
    drop(std::fs::remove_dir_all(&dir));
}

fn site(req: &leyline::testing::Request) -> testing::Response {
    match req.target.as_str() {
        "/start" => testing::Response::new(302).header("location", "/page"),
        _ => testing::Response::new(200),
    }
}

#[tokio::test]
async fn a_tab_tracks_the_page_for_later_requests() {
    let server = testing::Server::http(site).await.unwrap();
    let tab = Session::builder()
        .browser(Browser::default())
        .protocol(leyline::Protocol::Http1)
        .build()
        .unwrap()
        .tab();
    tab.open(server.url("/start")).await.unwrap();
    assert_eq!(tab.current().map(String::from), Some(server.url("/page")));
    tab.fetch(leyline::http::Method::GET, "/api")
        .send()
        .await
        .unwrap();
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

#[test]
fn a_loaded_session_takes_builder_settings() {
    let dir = scratch("loadset");
    let path = dir.join("device.json");
    let session = Session::builder()
        .browser(Browser::Chrome154)
        .languages(["de-DE", "de"])
        .build()
        .unwrap();
    session.save(&path).unwrap();
    let reopened = Session::load(&path)
        .unwrap()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    assert_eq!(reopened.languages(), session.languages());
    drop(std::fs::remove_dir_all(&dir));
}
