use super::super::Session;
use super::brand::{branded, capture_navigate_headers};
use crate::profile::{Browser, ChromiumBrand, Platform};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn brand_overlay_on_non_chromium_profile_is_err() {
    let err = Session::builder()
        .browser(Browser::Firefox148)
        .brand(ChromiumBrand::Edge)
        .build()
        .expect_err("Edge overlay on Firefox must fail");
    let message = format!("{err}");
    assert!(message.contains("Chromium"), "unexpected config: {message}");
}

#[test]
fn edge_keeps_explicit_chromium_browser() {
    let s = Session::builder()
        .browser(Browser::Chrome147)
        .brand(ChromiumBrand::Edge)
        .build()
        .unwrap();
    assert_eq!(s.browser(), Some(Browser::Chrome147));
    assert_eq!(s.brand(), Some(ChromiumBrand::Edge));
}

#[tokio::test]
async fn edge_overlay_on_chrome_145_preserves_chrome_145_grease_token() {
    let session = Session::builder()
        .browser(Browser::Chrome145)
        .brand(ChromiumBrand::Edge)
        .build()
        .unwrap();
    let req = capture_navigate_headers(session).await;
    let lower = req.to_lowercase();
    assert!(
        lower.contains(r#""microsoft edge";v="145""#),
        "Edge brand missing on 145 anchor:\n{req}"
    );
    assert!(
        lower.contains(r#""not:a-brand";v="99""#),
        "Chrome 145 GREASE placeholder (\"Not:A-Brand\";v=\"99\") missing \
             — overlay did not track active profile:\n{req}"
    );
    assert!(
        !lower.contains(r#""not.a/brand";v="8""#),
        "Overlay incorrectly emitted Chrome-147 GREASE on Chrome 145 anchor:\n{req}"
    );
}

#[test]
fn edge_overlay_on_mobile_platform_errors() {
    let err = Session::builder()
        .browser(Browser::Chrome147)
        .platform(Platform::Android)
        .brand(ChromiumBrand::Edge)
        .build()
        .expect_err("mobile Edge overlay must be rejected");
    assert!(format!("{err}").contains("not verified"));
}

#[test]
fn opera_overlay_on_unverified_anchor_errors() {
    let err = ChromiumBrand::Opera
        .overlay(144, Platform::Windows, "ua")
        .expect_err("Opera on Chromium 144 must be rejected until captured");
    assert!(format!("{err}").contains("not verified"));
}

#[test]
fn builder_edge_impersonates() {
    let s = branded(Browser::default_browser(), ChromiumBrand::Edge);
    assert_eq!(s.browser(), Some(Browser::default_browser()));
    assert_eq!(s.brand(), Some(ChromiumBrand::Edge));
}

#[test]
fn builder_opera_impersonates() {
    let s = branded(Browser::Chrome152, ChromiumBrand::Opera);
    assert_eq!(s.browser(), Some(Browser::Chrome152));
    assert_eq!(s.brand(), Some(ChromiumBrand::Opera));
}

#[test]
fn chrome_reports_stock_brand() {
    assert_eq!(Session::new().brand(), Some(ChromiumBrand::Chrome));
}

#[test]
fn firefox_and_bare_have_no_chromium_brand() {
    assert_eq!(
        Session::builder()
            .browser(Browser::latest(crate::profile::Family::Firefox))
            .build()
            .unwrap()
            .brand(),
        None
    );
    assert_eq!(Session::builder().build().unwrap().brand(), None);
}

#[tokio::test]
async fn user_dnt_override_wins_over_edge_default() {
    let session = branded(Browser::default_browser(), ChromiumBrand::Edge);
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
    let _ = session
        .request(http::Method::GET, format!("http://{addr}/"))
        .header("dnt", "0")
        .send()
        .await
        .unwrap();
    let req = server.await.unwrap();
    let lower = req.to_lowercase();
    assert!(
        lower.contains("\r\ndnt: 0\r\n"),
        "user's dnt=0 missing:\n{req}"
    );
    assert!(
        !lower.contains("\r\ndnt: 1\r\n"),
        "brand's dnt=1 leaked past user's override:\n{req}"
    );
}
