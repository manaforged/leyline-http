use super::super::Session;
use crate::profile::{Browser, ChromiumBrand, Platform, Preset};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// ---- ChromiumBrand overlay regression gates ----
// Each test drives a real GET through a local `TcpListener` mock
// and asserts the on-the-wire headers match the brand-specific
// shape captured from tls.peet.ws against real Edge 147, Brave
// 147, and Opera 129 browsers. If any overlay rule drifts from
// the captures, these fail.

async fn capture_navigate_headers(session: Session) -> String {
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
        .get(&format!("http://{addr}/"))
        .preset(Preset::Navigate)
        .send()
        .await
        .unwrap();
    server.await.unwrap()
}

#[tokio::test]
async fn edge_brand_overlay_matches_capture() {
    let session = Session::edge_latest().unwrap();
    let req = capture_navigate_headers(session).await;
    // UA suffix from real Edge 147 capture.
    assert!(
        req.contains("User-Agent: ")
            && req.contains("Chrome/147.0.0.0 Safari/537.36 Edg/147.0.0.0"),
        "Edge UA suffix missing:\n{req}"
    );
    // sec-ch-ua brand list reorders to put "Microsoft Edge" first.
    assert!(
        req.contains(r#"Sec-Ch-Ua: "Microsoft Edge";v="147""#)
            || req.contains(r#"sec-ch-ua: "Microsoft Edge";v="147""#),
        "Edge sec-ch-ua brand missing:\n{req}"
    );
    // Edge ships `dnt: 1` by default; Chrome does not.
    assert!(
        req.to_lowercase().contains("\r\ndnt: 1\r\n"),
        "Edge dnt missing:\n{req}"
    );
}

#[tokio::test]
async fn brave_brand_overlay_matches_capture() {
    let session = Session::brave_latest().unwrap();
    let req = capture_navigate_headers(session).await;
    // Brave keeps Chrome's UA byte-for-byte — no `Edg/` or `OPR/` suffix.
    assert!(
        !req.contains("Edg/") && !req.contains("OPR/"),
        "Brave UA must not contain sibling suffixes:\n{req}"
    );
    // Brand list reorders to put "Brave" first.
    assert!(
        req.to_lowercase().contains(r#"sec-ch-ua: "brave";v="147""#),
        "Brave sec-ch-ua brand missing:\n{req}"
    );
    // Brave ships sec-gpc: 1 for Global Privacy Control.
    assert!(
        req.to_lowercase().contains("\r\nsec-gpc: 1\r\n"),
        "Brave sec-gpc missing:\n{req}"
    );
    // Brave drops application/signed-exchange from the Navigate accept.
    assert!(
        !req.to_lowercase().contains("application/signed-exchange"),
        "Brave navigate accept should drop signed-exchange:\n{req}"
    );
}

#[tokio::test]
async fn opera_brand_overlay_matches_capture() {
    let session = Session::opera_latest().unwrap();
    let req = capture_navigate_headers(session).await;
    // Opera 129 against Chromium 145 per the lag table.
    assert!(
        req.contains("Chrome/145.0.0.0 Safari/537.36 OPR/129.0.0.0"),
        "Opera UA suffix missing:\n{req}"
    );
    // Opera uses "Not:A-Brand";v="99" (dash/colon/99) — not the
    // Chrome "Not.A/Brand";v="8" form. This distinguishes real
    // Opera captures from Chrome captures.
    assert!(
        req.to_lowercase()
            .contains(r#"sec-ch-ua: "not:a-brand";v="99", "opera";v="129""#),
        "Opera sec-ch-ua placeholder form missing:\n{req}"
    );
    assert!(
        req.to_lowercase().contains(r#""chromium";v="145""#),
        "Opera Chromium anchor 145 missing:\n{req}"
    );
}

#[tokio::test]
async fn chrome_default_has_no_brand_overlay() {
    // Smoke check: stock Chrome session should NOT ship any of
    // the sibling-brand-specific headers. If it does, the
    // overlay's gate in `SessionBuilder::build` leaked.
    let session = Session::chrome_latest().unwrap();
    let req = capture_navigate_headers(session).await;
    assert!(!req.contains("Edg/"));
    assert!(!req.contains("OPR/"));
    assert!(!req.to_lowercase().contains("\r\ndnt: 1\r\n"));
    assert!(!req.to_lowercase().contains("\r\nsec-gpc: 1\r\n"));
    assert!(
        req.to_lowercase().contains(r#"sec-ch-ua: "google chrome""#),
        "Chrome sec-ch-ua should identify as Google Chrome:\n{req}"
    );
}

#[tokio::test]
async fn brand_overlay_on_non_chromium_profile_is_noop() {
    // Setting `.brand(Edge)` on a Firefox profile must not
    // corrupt Firefox's identity — the overlay gate checks
    // `browser.chromium_major()` which returns None for Firefox.
    let session = Session::builder()
        .browser(Browser::Firefox148)
        .brand(ChromiumBrand::Edge)
        .build()
        .unwrap();
    let req = capture_navigate_headers(session).await;
    assert!(
        !req.contains("Edg/"),
        "Edge suffix leaked onto Firefox UA:\n{req}"
    );
    assert!(!req.to_lowercase().contains("\r\ndnt: 1\r\n"));
    assert!(req.to_lowercase().contains("firefox"));
}

#[tokio::test]
async fn edge_overlay_on_chrome_145_preserves_chrome_145_grease_token() {
    // Chrome 145 ships `"Not_A Brand";v="24"` (space, v=24), not
    // the Chrome-147 `"Not.A/Brand";v="8"` form. The overlay must
    // derive its sec-ch-ua from the profile, not a hardcoded
    // 147-era template. This test closes
    // a BLOCKER: the earlier overlay hardcoded v="8" regardless
    // of anchor, so `.browser(Chrome145).brand(Edge)` shipped a
    // provably-wrong sec-ch-ua.
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
        lower.contains(r#""not_a brand";v="24""#),
        "Chrome 145 GREASE placeholder (\"Not_A Brand\";v=\"24\") missing \
             — overlay did not track active profile:\n{req}"
    );
    assert!(
        !lower.contains(r#""not.a/brand";v="8""#),
        "Overlay incorrectly emitted Chrome-147 GREASE on Chrome 145 anchor:\n{req}"
    );
}

#[test]
fn edge_overlay_on_mobile_platform_errors() {
    // Real Edge for Android uses `EdgA/N` (note the trailing A),
    // not `Edg/N`. We haven't captured it; emitting `Edg/N` on
    // an Android UA is a detectable mismatch. The builder must
    // refuse instead of guess.
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
    // Only Chrome 145 / Opera 129 is verified by live capture.
    // Opera 130 shipped on Chromium 146 but we haven't captured
    // the sec-ch-ua GREASE placeholder for it — emitting the
    // 145-era `"Not:A-Brand";v="99"` form on Chromium 146+ is a
    // guess. Build must error until a real capture lands.
    let err = Session::builder()
        .browser(Browser::Chrome147)
        .brand(ChromiumBrand::Opera)
        .build()
        .expect_err("Opera on Chrome 147 must be rejected until captured");
    assert!(format!("{err}").contains("not verified"));
}

#[tokio::test]
async fn user_dnt_override_wins_over_edge_default() {
    // A caller on an Edge session who explicitly sets dnt=0 MUST
    // NOT see both `dnt: 1` (brand default) and `dnt: 0` (user)
    // on the wire. Before the fix the two headers both shipped
    // and some intermediaries coalesced them to `dnt: 1, 0`.
    let session = Session::edge_latest().unwrap();
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
        .get(&format!("http://{addr}/"))
        .preset(Preset::Navigate)
        .append_header("dnt", "0")
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
