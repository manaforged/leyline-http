use std::time::{Duration, Instant};

use leyline::cookie::Jar;
#[cfg(feature = "html")]
use leyline::html::{self, FormMethod};
use leyline::testing::{RecordedRequest, TestResponse, TestServer};
use leyline::{Browser, Device, Kind, Platform, Session};

#[cfg(feature = "html")]
const LOGIN_PAGE: &str = r#"<html><body>
<form id="signin" action="/session" method="post">
  <input type="hidden" name="csrf" value="tok&amp;en">
  <input name="email" value="">
  <input type="checkbox" name="remember" checked>
  <input type="checkbox" name="news">
  <select name="locale"><option value="en">EN</option><option value="de" selected>DE</option></select>
  <textarea name="note">
hi</textarea>
  <button type="submit" name="go" value="1">Sign in</button>
</form>
<script>var x = "<form action='/fake'>";</script>
</body></html>"#;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("leyline-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(feature = "html")]
#[test]
fn forms_parse_hidden_fields_and_controls() {
    let found = html::forms(LOGIN_PAGE);
    assert_eq!(found.len(), 1);
    let form = &found[0];
    assert_eq!(form.action(), "/session");
    assert_eq!(form.method(), FormMethod::Post);
    assert_eq!(form.field("csrf"), Some("tok&en"));
    assert_eq!(form.field("remember"), Some("on"));
    assert_eq!(form.field("news"), None);
    assert_eq!(form.field("locale"), Some("de"));
    assert_eq!(form.field("note"), Some("hi"));
    assert_eq!(form.buttons(), ["go"]);
    assert!(html::Form::find(LOGIN_PAGE, "signin").is_some());
}

#[cfg(feature = "html")]
fn site(req: &RecordedRequest) -> TestResponse {
    match req.target.as_str() {
        "/login" => TestResponse::new(200).body(LOGIN_PAGE),
        _ => TestResponse::new(200),
    }
}

#[cfg(feature = "html")]
#[tokio::test]
async fn a_tab_submits_a_parsed_form_like_a_click() {
    let server = TestServer::http(site).await.unwrap();
    let tab = Session::builder()
        .browser(Browser::default())
        .protocol(leyline::ProtocolPolicy::Http1)
        .build()
        .unwrap()
        .tab();
    let page = tab.open(server.url("/login")).await.unwrap();
    let mut form = html::Form::find(&page.text().await.unwrap(), "signin").unwrap();
    form.set("email", "a@shop.example");
    tab.submit_form(&form).await.unwrap();

    let posted = server
        .requests()
        .await
        .into_iter()
        .find(|r| r.target == "/session")
        .unwrap();
    assert_eq!(posted.method, "POST");
    let body = String::from_utf8(posted.body.clone()).unwrap();
    assert!(body.contains("csrf=tok%26en"), "{body}");
    assert!(body.contains("email=a%40shop.example"), "{body}");
    assert_eq!(posted.header("sec-fetch-mode"), Some("navigate"));
    assert_eq!(
        posted.header("referer"),
        Some(server.url("/login").as_str())
    );
}

#[test]
fn an_unpinned_device_still_refuses_a_different_session() {
    let session = Session::builder()
        .browser(Browser::Chrome154)
        .platform(Platform::Windows)
        .build()
        .unwrap();
    let mut device = Device::capture(&session, None);
    device.profile_id = None;
    device
        .app
        .insert("email".to_owned(), serde_json::json!("a@shop.example"));
    let json = serde_json::to_string(&device).unwrap();
    let device: Device = serde_json::from_str(&json).unwrap();
    assert_eq!(device.app["email"], "a@shop.example");
    device.check(&session).unwrap();

    let moved = Session::builder()
        .browser(Browser::Chrome154)
        .platform(Platform::MacOS)
        .build()
        .unwrap();
    assert_eq!(device.check(&moved).unwrap_err().kind(), Kind::Config);
}

#[tokio::test]
async fn device_autosave_writes_the_device_and_its_jar() {
    let dir = scratch("devsave");
    let session = Session::builder()
        .browser(Browser::Chrome154)
        .build()
        .unwrap();
    let mut device = Device::capture(&session, None);
    device.jar_path = Some(dir.join("jar.json"));
    let path = dir.join("device.json");
    let autosave = device.autosave(&session, &path, Duration::from_millis(20));
    let url = url::Url::parse("https://shop.example/").unwrap();
    session
        .cookies()
        .store_set_cookie("sid=1; Path=/; Max-Age=3600", &url);
    autosave.flush().await.unwrap();
    assert!(Device::load_from(&path).is_ok());
    let jar = Jar::load_from(dir.join("jar.json")).unwrap();
    assert_eq!(jar.get_cookie(&url, "sid").as_deref(), Some("1"));
    autosave.shutdown().await.unwrap();
    drop(std::fs::remove_dir_all(&dir));
}

#[tokio::test(flavor = "multi_thread")]
#[expect(
    clippy::disallowed_methods,
    reason = "the handler must block its thread to prove the test server isolates it"
)]
async fn a_blocking_test_handler_does_not_stall_client_timeouts() {
    let server = TestServer::http(|_| {
        std::thread::sleep(Duration::from_secs(2));
        TestResponse::new(200)
    })
    .await
    .unwrap();
    let started = Instant::now();
    let err = Session::builder()
        .build()
        .unwrap()
        .get(server.url("/slow"))
        .timeout(Duration::from_millis(200))
        .await
        .unwrap_err();
    assert!(err.is_timeout());
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn test_responses_can_wait_and_arrive_in_chunks() {
    let server = TestServer::http(|req: &RecordedRequest| {
        if req.target == "/slow" {
            TestResponse::new(200).delay(Duration::from_secs(2))
        } else {
            TestResponse::new(200).chunks(["ab", "cd", "ef"], Duration::from_millis(10))
        }
    })
    .await
    .unwrap();
    let session = Session::builder().build().unwrap();
    let err = session
        .get(server.url("/slow"))
        .timeout(Duration::from_millis(200))
        .await
        .unwrap_err();
    assert!(err.is_timeout());
    let text = session
        .get(server.url("/chunks"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(text, "abcdef");
}

#[tokio::test]
async fn device_autosave_saves_updates_to_the_device() {
    let dir = scratch("devupdate");
    let session = Session::browser(Browser::Chrome154);
    let device = Device::capture(&session, None);
    let path = dir.join("device.json");
    let autosave = device.autosave(&session, &path, Duration::from_millis(20));
    autosave.update(|device| {
        device.app.insert(
            "last_page".to_owned(),
            serde_json::json!("https://shop.example/cart"),
        );
    });
    autosave.flush().await.unwrap();
    let saved = Device::load_from(&path).unwrap();
    assert_eq!(saved.app["last_page"], "https://shop.example/cart");
    autosave.shutdown().await.unwrap();
    drop(std::fs::remove_dir_all(&dir));
}

#[test]
fn a_device_refuses_other_languages_or_another_proxy() {
    let session = Session::builder()
        .browser(Browser::Chrome154)
        .languages(["de-DE", "de"])
        .proxy("http://proxy-a.example:8080")
        .build()
        .unwrap();
    let device = Device::capture(&session, session.proxy_url());
    device.check(&session).unwrap();

    let english = Session::builder()
        .browser(Browser::Chrome154)
        .languages(["en-US", "en"])
        .proxy("http://proxy-a.example:8080")
        .build()
        .unwrap();
    let err = device.check(&english).unwrap_err();
    assert!(err.to_string().contains("languages"), "{err}");

    let moved = session.with_proxy("http://proxy-b.example:8080");
    let err = device.check(&moved).unwrap_err();
    assert!(err.to_string().contains("proxy"), "{err}");
}

#[cfg(not(feature = "socks"))]
#[test]
fn a_socks_proxy_in_a_pool_needs_the_feature_at_build() {
    let err = Session::builder()
        .proxy_pool(leyline::ProxyPool::new(["socks5://127.0.0.1:1080"]))
        .build()
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Config);
    assert!(err.to_string().contains("socks"), "{err}");
}

#[cfg(feature = "html")]
#[test]
fn pages_give_meta_tokens_and_links() {
    let page = r#"<html><head>
<meta name="CSRF-Token" content="a&amp;b">
<meta property="og:title" content="Cart">
</head><body>
<script>var s = "<a href='/fake'>x</a>";</script>
<a href="/p/1" rel="next">First
  product</a>
<a name="anchor">no href</a>
</body></html>"#;
    assert_eq!(html::meta(page, "csrf-token").as_deref(), Some("a&b"));
    assert_eq!(html::meta(page, "og:title").as_deref(), Some("Cart"));
    let links = html::links(page);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].href(), "/p/1");
    assert_eq!(links[0].text(), "First product");
    assert_eq!(links[0].rel(), Some("next"));
}

#[test]
fn a_strict_device_needs_a_pin_and_a_proxy() {
    let session = Session::browser(Browser::Chrome154);
    let mut device = Device::capture(&session, None);
    device.check(&session).unwrap();
    device.strict = true;
    device.profile_id = None;
    let err = device.check(&session).unwrap_err();
    assert_eq!(err.kind(), Kind::Config);
    let text = err.to_string();
    assert!(
        text.contains("profile_id") && text.contains("proxy"),
        "{text}"
    );
}

#[tokio::test]
async fn device_autosave_tracks_the_tab_page() {
    let dir = scratch("devtab");
    let server = TestServer::http(|_| TestResponse::new(200)).await.unwrap();
    let session = Session::browser(Browser::Chrome154);
    let device = Device::capture(&session, None);
    let path = dir.join("device.json");
    let autosave = device.autosave(&session, &path, Duration::from_millis(20));
    let tab = session.tab();
    autosave.track(&tab);
    tab.open(server.url("/cart")).await.unwrap();
    assert_eq!(
        autosave.device().page.map(String::from),
        Some(server.url("/cart"))
    );
    autosave.flush().await.unwrap();
    let saved = Device::load_from(&path).unwrap();
    let restored = saved.tab(&session);
    assert_eq!(
        restored.current().map(String::from),
        Some(server.url("/cart"))
    );
    autosave.shutdown().await.unwrap();
    drop(std::fs::remove_dir_all(&dir));
}
