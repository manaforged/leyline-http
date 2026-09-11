use crate::Session;
use crate::profile::{Browser, Platform, Preset};
use crate::{Kind, Request, RequestBuilder};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn capture_get_headers(session: Session) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut req = Vec::new();
        let mut tmp = [0u8; 2048];
        loop {
            let n = sock.read(&mut tmp).await.unwrap();
            if n == 0 {
                break;
            }
            req.extend_from_slice(&tmp[..n]);
            if req.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
            .await
            .unwrap();
        String::from_utf8_lossy(&req).to_string()
    });
    let _ = session.get(&format!("http://{addr}/")).await.unwrap();
    server.await.unwrap()
}

#[test]
fn default_session_is_bare() {
    assert_eq!(Session::new().browser(), None);
    assert_eq!(Session::builder().build().unwrap().browser(), None);
}

#[test]
fn chrome_helper_is_explicit_browser() {
    assert_eq!(
        Session::chrome().browser(),
        Some(Browser::default_browser())
    );
}

#[test]
fn bare_default_platform_follows_host() {
    let s = Session::new();
    assert_eq!(s.platform(), Platform::detect_host());
    let chrome = Session::builder()
        .browser(Browser::Chrome148)
        .build()
        .unwrap();
    assert_eq!(chrome.platform(), Platform::Windows);
}

#[tokio::test]
async fn bare_session_sends_generic_ua_and_no_client_hints() {
    let req = capture_get_headers(Session::new()).await;
    let lower = req.to_lowercase();
    assert!(
        lower.contains("user-agent: leyline/"),
        "bare UA should be leyline/<version>:\n{req}"
    );
    assert!(
        !lower.contains("sec-ch-ua"),
        "bare session must not emit client-hint headers:\n{req}"
    );
    assert!(
        !lower.contains("chrome") && !lower.contains("mozilla"),
        "bare session must not look like a browser:\n{req}"
    );
}

#[tokio::test]
async fn chrome_get_emits_navigate_headers() {
    let req = capture_get_headers(Session::chrome()).await;
    let lower = req.to_lowercase();
    assert!(
        lower.contains("sec-fetch-mode: navigate"),
        "Chrome GET must look like a document fetch:\n{req}"
    );
    assert!(
        lower.contains("sec-ch-ua"),
        "Chrome GET must emit client hints:\n{req}"
    );
}

#[test]
fn session_retry_default_is_inherited_by_requests() {
    use crate::RetryPolicy;
    let policy = RetryPolicy::transient().with_max_retries(7);
    let session = Session::builder().retry(policy).build().unwrap();
    let req = session.request(http::Method::GET, "https://example.test/");
    assert_eq!(req.retry_policy.max_retries, 7);
    let overridden = session
        .request(http::Method::GET, "https://example.test/")
        .retry(RetryPolicy::none());
    assert_eq!(overridden.retry_policy.max_retries, 0);
    let bare = Session::new().request(http::Method::GET, "https://example.test/");
    assert_eq!(bare.retry_policy.max_retries, 0);
}

#[test]
fn platform_host_resolves_and_never_leaks() {
    assert_eq!(Platform::Host.resolve(), Platform::detect_host());
    assert_ne!(Platform::detect_host(), Platform::Host);
    let s = Session::builder()
        .browser(Browser::Chrome148)
        .macos()
        .build()
        .unwrap();
    assert_eq!(s.platform(), Platform::MacOS);
}

#[test]
fn chrome_linux_is_one_chain() {
    let s = Session::builder().chrome().linux().build().unwrap();
    assert_eq!(s.browser(), Some(Browser::default_browser()));
    assert_eq!(s.platform(), Platform::Linux);
}

#[test]
fn safari_ios_picks_iphone_profile() {
    let s = Session::builder().safari().ios().build().unwrap();
    assert_eq!(s.browser(), Some(Browser::SafariIOS18));
    assert_eq!(s.platform(), Platform::IOS);
}

#[test]
fn ios_then_safari_still_iphone() {
    let s = Session::builder().ios().safari().build().unwrap();
    assert_eq!(s.browser(), Some(Browser::SafariIOS18));
    assert_eq!(s.platform(), Platform::IOS);
}

#[test]
fn safari_windows_profile_is_err() {
    let err = Session::profile(Browser::Safari26, Platform::Windows)
        .expect_err("safari on windows is not a profile");
    assert_eq!(err.kind(), Kind::Config, "expected Config, got {err:?}");
    let message = err.message().expect("config errors carry a message");
    assert!(message.contains("Windows"), "unexpected config: {message}");
}

#[test]
fn windows_then_brave_keeps_windows() {
    let s = Session::builder().windows().brave().build().unwrap();
    assert_eq!(s.browser(), Some(Browser::Brave146));
    assert_eq!(s.platform(), Platform::Windows);
}

async fn capture_post_headers(
    session: Session,
    finish: impl FnOnce(RequestBuilder) -> RequestBuilder,
) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut req = Vec::new();
        let mut tmp = [0u8; 2048];
        loop {
            let n = sock.read(&mut tmp).await.unwrap();
            if n == 0 {
                break;
            }
            req.extend_from_slice(&tmp[..n]);
            if req.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
            .await
            .unwrap();
        String::from_utf8_lossy(&req).to_string()
    });
    let _ = finish(session.post(&format!("http://{addr}/")))
        .await
        .unwrap();
    server.await.unwrap()
}

#[tokio::test]
async fn execute_json_content_type_infers_xhr() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut req = Vec::new();
        let mut tmp = [0u8; 2048];
        loop {
            let n = sock.read(&mut tmp).await.unwrap();
            if n == 0 {
                break;
            }
            req.extend_from_slice(&tmp[..n]);
            if req.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
            .await
            .unwrap();
        String::from_utf8_lossy(&req).to_string()
    });
    let req = Request::new(http::Method::POST, format!("http://{addr}/"))
        .header("content-type", "application/json")
        .body("{}");
    Session::chrome().execute(req).await.unwrap();
    let wire = server.await.unwrap();
    let lower = wire.to_lowercase();
    assert!(
        lower.contains("sec-fetch-mode: cors"),
        "owned Request JSON POST should look like XHR:\n{wire}"
    );
}

#[tokio::test]
async fn json_body_infers_xhr_preset() {
    let req =
        capture_post_headers(Session::chrome(), |b| b.json(&serde_json::json!({"a": 1}))).await;
    let lower = req.to_lowercase();
    assert!(
        lower.contains("sec-fetch-mode: cors"),
        "JSON POST should look like XHR:\n{req}"
    );
    assert!(
        lower.contains("content-type: application/json"),
        "JSON POST must set content-type:\n{req}"
    );
}

#[tokio::test]
async fn form_body_infers_form_preset() {
    let req = capture_post_headers(Session::chrome(), |b| b.form([("u", "alice")])).await;
    let lower = req.to_lowercase();
    assert!(
        lower.contains("sec-fetch-mode: cors"),
        "form POST should look like Form:\n{req}"
    );
    assert!(
        lower.contains("content-type: application/x-www-form-urlencoded"),
        "form POST must set content-type:\n{req}"
    );
}

#[tokio::test]
async fn user_preset_wins_over_json_inference() {
    let req = capture_post_headers(Session::chrome(), |b| {
        b.preset(Preset::Navigate)
            .json(&serde_json::json!({"a": 1}))
    })
    .await;
    let lower = req.to_lowercase();
    assert!(
        lower.contains("sec-fetch-mode: navigate"),
        "explicit preset must not be overwritten by json():\n{req}"
    );
}

#[tokio::test]
async fn bare_json_has_no_sec_fetch() {
    let req = capture_post_headers(Session::new(), |b| b.json(&serde_json::json!({"a": 1}))).await;
    let lower = req.to_lowercase();
    assert!(
        !lower.contains("sec-fetch-"),
        "bare JSON POST must not impersonate:\n{req}"
    );
}

#[tokio::test]
async fn native_json_omits_browser_headers_and_preserves_app_headers() {
    let session = Session::chrome();
    let wire = capture_post_headers(session, |request| {
        request
            .preset(Preset::Native)
            .header("user-agent", "ExampleApp/1.0")
            .header("authorization", "Bearer test-token")
            .json(&serde_json::json!({"id": 1}))
    })
    .await;
    let lower = wire.to_lowercase();
    for name in ["origin:", "referer:", "sec-fetch-", "sec-ch-ua"] {
        assert!(
            !lower.contains(name),
            "native request contains {name}: {wire}"
        );
    }
    assert!(lower.contains("user-agent: exampleapp/1.0"));
    assert!(lower.contains("authorization: bearer test-token"));
    assert!(lower.contains("content-type: application/json"));
}
