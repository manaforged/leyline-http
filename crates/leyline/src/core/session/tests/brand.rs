use super::super::Session;
use crate::profile::{Browser, ChromiumBrand, Platform};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(super) fn branded(browser: Browser, brand: ChromiumBrand) -> Session {
    Session::builder()
        .browser(browser)
        .brand(brand)
        .build()
        .unwrap()
}

pub(super) async fn capture_navigate_headers(session: Session) -> String {
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
    let _ = session.get(format!("http://{addr}/")).await.unwrap();
    server.await.unwrap()
}

#[tokio::test]
async fn identity_keeps_ua_when_tls_rotates() {
    let id = crate::Identity::locked(Browser::Chrome150, Platform::Windows)
        .rotate_tls(Browser::Chrome146)
        .expect("same family");
    let mixed = Session::builder().identity(id).build().unwrap();
    let headers = capture_navigate_headers(mixed).await;
    assert!(
        headers.contains("Chrome/150"),
        "HTTP identity must stay Chrome 150:\n{headers}"
    );
    assert!(
        !headers.contains("Chrome/146"),
        "TLS roll must not rewrite UA to 146:\n{headers}"
    );
}

#[tokio::test]
async fn brand_overlay_follows_http_major_not_tls() {
    let id = crate::Identity::locked(Browser::Chrome150, Platform::Windows)
        .rotate_tls(Browser::Chrome145)
        .expect("same family");
    let session = Session::builder()
        .identity(id)
        .brand(ChromiumBrand::Edge)
        .build()
        .unwrap();
    let headers = capture_navigate_headers(session).await;
    assert!(
        headers.contains("Chrome/150") && headers.contains("Edg/150"),
        "Edge overlay must stamp the HTTP major:\n{headers}"
    );
    assert!(
        !headers.contains("Edg/116") && !headers.contains("Chrome/116"),
        "TLS hello must not drive brand or UA:\n{headers}"
    );
}

#[tokio::test]
async fn edge_brand_overlay_matches_capture() {
    let session = branded(Browser::Chrome152, ChromiumBrand::Edge);
    let req = capture_navigate_headers(session).await;
    assert!(
        req.contains("Chrome/152.0.0.0 Safari/537.36 Edg/152.0.0.0"),
        "Edge UA must carry reduced Edg/152.0.0.0 suffix:\n{req}"
    );
    assert!(
        req.contains(r#""Microsoft Edge";v="152""#),
        "Edge sec-ch-ua brand missing:\n{req}"
    );
    assert!(
        !req.to_lowercase().contains("\r\ndnt:"),
        "Edge must not send dnt by default:\n{req}"
    );
}

#[tokio::test]
async fn brave_first_class_profile_matches_capture() {
    let session = Session::builder()
        .browser(Browser::Brave146)
        .platform(Platform::MacOS)
        .build()
        .unwrap();
    let req = capture_navigate_headers(session).await;
    assert!(
        !req.contains("Edg/") && !req.contains("OPR/"),
        "Brave UA must not contain sibling suffixes:\n{req}"
    );
    assert!(
        req.to_lowercase()
            .contains(r#"sec-ch-ua: "chromium";v="146", "not-a.brand";v="24", "brave";v="146""#),
        "Brave sec-ch-ua slot order / GREASE form missing:\n{req}"
    );
    assert!(
        req.to_lowercase().contains("\r\nsec-gpc: 1\r\n"),
        "Brave sec-gpc missing:\n{req}"
    );
    assert!(
        !req.to_lowercase().contains("application/signed-exchange"),
        "Brave navigate accept should drop signed-exchange:\n{req}"
    );
    assert!(
        req.to_lowercase()
            .contains("accept-language: en-us,en;q=0.8"),
        "Brave accept-language q=0.8 missing:\n{req}"
    );
    let lower = req.to_lowercase();
    let pos = |needle: &str| lower.find(needle);
    let p_sec_gpc = pos("\r\nsec-gpc:").unwrap_or(usize::MAX);
    let p_lang = pos("\r\naccept-language:").unwrap_or(usize::MAX);
    let p_site = pos("\r\nsec-fetch-site:").unwrap_or(usize::MAX);
    assert!(
        p_sec_gpc < p_lang && p_lang < p_site,
        "Brave header order wrong (sec-gpc < accept-language < sec-fetch-site):\n{req}"
    );
}

#[tokio::test]
async fn opera_brand_overlay_matches_capture() {
    let session = branded(Browser::Chrome152, ChromiumBrand::Opera);
    let req = capture_navigate_headers(session).await;
    assert!(
        req.contains("Chrome/152.0.0.0 Safari/537.36 OPR/136.0.0.0"),
        "Opera UA suffix missing:\n{req}"
    );
    assert!(
        req.to_lowercase()
            .contains(r#"sec-ch-ua: "chromium";v="152", "not?a_brand";v="24", "opera";v="136""#),
        "Opera sec-ch-ua missing:\n{req}"
    );
    assert!(
        req.to_lowercase().contains(r#""chromium";v="152""#),
        "Opera Chromium anchor 152 missing:\n{req}"
    );
}

#[tokio::test]
async fn opera_146_overlay_emits_130() {
    let session = Session::builder()
        .browser(Browser::Chrome146)
        .brand(ChromiumBrand::Opera)
        .build()
        .unwrap();
    let req = capture_navigate_headers(session).await;
    assert!(
        req.contains("Chrome/146.0.0.0 Safari/537.36 OPR/130.0.0.0"),
        "Opera 146 UA suffix missing:\n{req}"
    );
    assert!(
        req.to_lowercase()
            .contains(r#"sec-ch-ua: "chromium";v="146", "not-a.brand";v="24", "opera";v="130""#),
        "Opera 146 sec-ch-ua missing:\n{req}"
    );
}

#[tokio::test]
async fn opera_148_overlay_emits_132() {
    let session = Session::builder()
        .browser(Browser::Chrome148)
        .brand(ChromiumBrand::Opera)
        .build()
        .unwrap();
    let req = capture_navigate_headers(session).await;
    assert!(
        req.contains("Chrome/148.0.0.0 Safari/537.36 OPR/132.0.0.0"),
        "Opera 148 UA suffix missing:\n{req}"
    );
    assert!(
        req.to_lowercase()
            .contains(r#"sec-ch-ua: "chromium";v="148", "opera";v="132", "not/a)brand";v="99""#),
        "Opera 148 sec-ch-ua missing:\n{req}"
    );
}

#[tokio::test]
async fn opera_150_overlay_emits_134() {
    let session = Session::builder()
        .browser(Browser::Chrome150)
        .brand(ChromiumBrand::Opera)
        .build()
        .unwrap();
    let req = capture_navigate_headers(session).await;
    assert!(
        req.contains("Chrome/150.0.0.0 Safari/537.36 OPR/134.0.0.0"),
        "Opera 150 UA suffix missing:\n{req}"
    );
    assert!(
        req.to_lowercase()
            .contains(r#"sec-ch-ua: "not;a=brand";v="8", "chromium";v="150", "opera";v="134""#),
        "Opera 150 sec-ch-ua missing:\n{req}"
    );
}

#[tokio::test]
async fn opera_145_overlay_still_supported() {
    let session = Session::builder()
        .browser(Browser::Chrome145)
        .brand(ChromiumBrand::Opera)
        .build()
        .unwrap();
    let req = capture_navigate_headers(session).await;
    assert!(
        req.contains("Chrome/145.0.0.0 Safari/537.36 OPR/129.0.0.0"),
        "Opera 145 backward-compat UA suffix missing:\n{req}"
    );
    assert!(
        req.to_lowercase()
            .contains(r#""opera";v="129", "chromium";v="145""#),
        "Opera 145 sec-ch-ua missing:\n{req}"
    );
}

#[tokio::test]
async fn vivaldi_brand_overlay_matches_capture() {
    let session = branded(Browser::Chrome147, ChromiumBrand::Vivaldi);
    let req = capture_navigate_headers(session).await;

    assert!(
        req.contains("Chrome/147.0.0.0 Safari/537.36 Vivaldi/7.9."),
        "Vivaldi UA suffix missing:\n{req}"
    );
    let lower = req.to_lowercase();
    let sec_ch_ua_line = req
        .lines()
        .find(|l| l.to_lowercase().starts_with("sec-ch-ua:"))
        .expect("sec-ch-ua header present");
    assert!(
        !sec_ch_ua_line.to_lowercase().contains("google chrome"),
        "Vivaldi must drop \"Google Chrome\" from sec-ch-ua:\n{sec_ch_ua_line}"
    );
    assert!(
        !sec_ch_ua_line.to_lowercase().contains("vivaldi"),
        "Vivaldi must NOT advertise itself in sec-ch-ua (per vendor docs):\n{sec_ch_ua_line}"
    );
    assert!(
        lower.contains(r#""chromium";v="147""#),
        "Vivaldi sec-ch-ua should still carry Chromium anchor:\n{req}"
    );
    assert!(
        !lower.contains("\r\ndnt:"),
        "Vivaldi must not ship dnt by default:\n{req}"
    );
    assert!(
        !lower.contains("\r\nsec-gpc:"),
        "Vivaldi must not ship sec-gpc by default:\n{req}"
    );
}

#[test]
fn vivaldi_overlay_on_unverified_anchor_errors() {
    for bad in [144u32, 145, 146, 148, 150] {
        let err = ChromiumBrand::Vivaldi
            .overlay(bad, Platform::Windows, "ua")
            .expect_err(&format!(
                "Vivaldi on Chromium {bad} must be rejected until captured"
            ));
        assert!(format!("{err}").contains("not verified"), "{err}");
    }
}

#[test]
fn vivaldi_overlay_on_mobile_errors() {
    for p in [Platform::Android, Platform::IOS] {
        let err = ChromiumBrand::Vivaldi
            .overlay(147, p, "ua")
            .expect_err("mobile Vivaldi overlay must be rejected");
        assert!(format!("{err}").contains("not verified"));
    }
}

#[tokio::test]
async fn chrome_default_has_no_brand_overlay() {
    let session = Session::new();
    let req = capture_navigate_headers(session).await;
    assert!(!req.contains("Edg/"));
    assert!(!req.contains("OPR/"));
    assert!(!req.to_lowercase().contains("\r\ndnt: 1\r\n"));
    assert!(!req.to_lowercase().contains("\r\nsec-gpc: 1\r\n"));
    let sec_ch_ua_line = req
        .lines()
        .find(|l| l.to_lowercase().starts_with("sec-ch-ua:"))
        .expect("sec-ch-ua header present")
        .to_lowercase();
    let major = Browser::default_browser().version();
    assert!(
        sec_ch_ua_line.contains(&format!(r#""google chrome";v="{major}""#)),
        "Chrome sec-ch-ua should identify as Google Chrome {major}:\n{req}"
    );
    assert!(
        !sec_ch_ua_line.contains("microsoft edge")
            && !sec_ch_ua_line.contains("opera")
            && !sec_ch_ua_line.contains("brave"),
        "stock Chrome sec-ch-ua must not carry a sibling-brand token:\n{req}"
    );
}
