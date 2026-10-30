use std::time::{Duration, Instant};

#[cfg(feature = "html")]
use leyline::html;
use leyline::testing;
use leyline::{Browser, Kind, Session};

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
    assert_eq!(form.method(), http::Method::POST);
    assert_eq!(form.field("csrf"), Some("tok&en"));
    assert_eq!(form.field("remember"), Some("on"));
    assert_eq!(form.field("news"), None);
    assert_eq!(form.field("locale"), Some("de"));
    assert_eq!(form.field("note"), Some("hi"));
    assert_eq!(form.buttons(), ["go"]);
    assert!(html::Form::find(LOGIN_PAGE, "signin").is_some());
}

#[cfg(feature = "html")]
fn site(req: &testing::Request) -> testing::Response {
    match req.target.as_str() {
        "/login" => testing::Response::new(200).body(LOGIN_PAGE),
        _ => testing::Response::new(200),
    }
}

#[cfg(feature = "html")]
#[tokio::test]
async fn a_tab_submits_a_parsed_form_like_a_click() {
    let server = testing::Server::http(site).await.unwrap();
    let tab = Session::builder()
        .browser(Browser::default())
        .protocol(leyline::Protocol::Http1)
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

async fn saved_cookie_soon(path: &std::path::Path, url: &url::Url) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let saved = Session::load(path)
            .ok()
            .and_then(|builder| builder.build().ok())
            .and_then(|session| session.cookies().get(url, "sid"));
        if saved.is_some() {
            return saved;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    None
}

#[tokio::test]
async fn autosave_writes_a_cookie_change() {
    let dir = scratch("devsave");
    let session = Session::builder()
        .browser(Browser::Chrome154)
        .build()
        .unwrap();
    let path = dir.join("device.json");
    session.autosave(&path, Duration::from_millis(20));
    let url = url::Url::parse("https://shop.example/").unwrap();
    session.cookies().store("sid=1; Path=/; Max-Age=3600", &url);
    assert_eq!(saved_cookie_soon(&path, &url).await.as_deref(), Some("1"));
    session.shutdown();
    drop(std::fs::remove_dir_all(&dir));
}

#[tokio::test]
async fn autosave_writes_a_final_save_on_shutdown() {
    let dir = scratch("devshutdown");
    let session = Session::from(Browser::Chrome154);
    let path = dir.join("device.json");
    session.autosave(&path, Duration::from_secs(3600));
    let url = url::Url::parse("https://shop.example/").unwrap();
    session.cookies().store("sid=2; Path=/; Max-Age=3600", &url);
    session.shutdown();
    assert_eq!(saved_cookie_soon(&path, &url).await.as_deref(), Some("2"));
    drop(std::fs::remove_dir_all(&dir));
}

#[tokio::test(flavor = "multi_thread")]
#[expect(
    clippy::disallowed_methods,
    reason = "the handler must block its thread to prove the test server isolates it"
)]
async fn a_blocking_test_handler_does_not_stall_client_timeouts() {
    let server = testing::Server::http(|_| {
        std::thread::sleep(Duration::from_secs(2));
        testing::Response::new(200)
    })
    .await
    .unwrap();
    let started = Instant::now();
    let err = Session::builder()
        .build()
        .unwrap()
        .get(server.url("/slow"))
        .timeout(Duration::from_millis(200))
        .send()
        .await
        .unwrap_err();
    assert!(err.kind() == Kind::Timeout);
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn test_responses_can_wait_and_arrive_in_chunks() {
    let server = testing::Server::http(|req: &testing::Request| {
        if req.target == "/slow" {
            testing::Response::new(200).delay(Duration::from_secs(2))
        } else {
            testing::Response::new(200).chunks(["ab", "cd", "ef"], Duration::from_millis(10))
        }
    })
    .await
    .unwrap();
    let session = Session::builder().build().unwrap();
    let err = session
        .get(server.url("/slow"))
        .timeout(Duration::from_millis(200))
        .send()
        .await
        .unwrap_err();
    assert!(err.kind() == Kind::Timeout);
    let text = session
        .get(server.url("/chunks"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(text, "abcdef");
}

#[cfg(not(feature = "socks"))]
#[test]
fn a_socks_proxy_in_a_pool_needs_the_feature_at_build() {
    let err = Session::builder()
        .proxy(leyline::Proxy::list(["socks5://127.0.0.1:1080"]))
        .build()
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Config);
    assert!(err.to_string().contains("socks"), "{err}");
}
