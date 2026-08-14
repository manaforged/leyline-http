use super::super::Session;
use crate::profile::{Browser, ChromiumBrand, Platform, Preset};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// ---- ChromiumBrand overlay regression gates ----
// Each test drives a real GET through a local `TcpListener` mock
// and asserts the on-the-wire headers match the brand-specific shape
// for that variant. If any overlay rule drifts, these fail.
//
// Verification provenance:
//   - Edge 147, Brave 147, Opera 129  → live captures from tls.peet.ws.
//   - Opera 130/131, Vivaldi 7.9      → vendor release notes,
//     structurally identical to the verified shape.

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
        .rotate_tls(Browser::Chrome116)
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
    let session = Session::edge();
    let req = capture_navigate_headers(session).await;
    // Real Edge ships the UA-reduced `Edg/{major}.0.0.0` form on
    // current versions. Verified against tls.peet.ws on 2026-04-25
    // with Edge 147 on macOS.
    assert!(
        req.contains("Chrome/147.0.0.0 Safari/537.36 Edg/147.0.0.0"),
        "Edge UA must carry reduced Edg/147.0.0.0 suffix:\n{req}"
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
async fn brave_first_class_profile_matches_capture() {
    // Brave 146 on macOS — Browser::Brave146 ships the full set of
    // captured deltas: header reorder, accept SXG-strip, sec-gpc,
    // accept-language q=0.8, custom sec-ch-ua slot order. Verified
    // against tls.peet.ws on 2026-04-25.
    let session = Session::brave();
    let req = capture_navigate_headers(session).await;
    // No Edg/ or OPR/ suffix — Brave keeps Chrome's UA suffix-free.
    assert!(
        !req.contains("Edg/") && !req.contains("OPR/"),
        "Brave UA must not contain sibling suffixes:\n{req}"
    );
    // Brave-specific sec-ch-ua: Chromium leads, then GREASE, then Brave.
    assert!(
        req.to_lowercase()
            .contains(r#"sec-ch-ua: "chromium";v="146", "not-a.brand";v="24", "brave";v="146""#),
        "Brave sec-ch-ua slot order / GREASE form missing:\n{req}"
    );
    // sec-gpc: 1 for Global Privacy Control.
    assert!(
        req.to_lowercase().contains("\r\nsec-gpc: 1\r\n"),
        "Brave sec-gpc missing:\n{req}"
    );
    // Navigate accept drops application/signed-exchange.
    assert!(
        !req.to_lowercase().contains("application/signed-exchange"),
        "Brave navigate accept should drop signed-exchange:\n{req}"
    );
    // accept-language uses q=0.8 (Brave specific).
    assert!(
        req.to_lowercase()
            .contains("accept-language: en-us,en;q=0.8"),
        "Brave accept-language q=0.8 missing:\n{req}"
    );
    // Header order: accept-language sits between sec-gpc and sec-fetch-site.
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
    // Session::opera() resolves to the latest verified Opera anchor —
    // currently Chrome 147 / Opera 131 (vendor-doc EXTRAPOLATED; see
    // OPERA_PER_CHROMIUM in profile::brand). Bump these assertions when the
    // anchor table moves.
    let session = Session::opera();
    let req = capture_navigate_headers(session).await;
    assert!(
        req.contains("Chrome/147.0.0.0 Safari/537.36 OPR/131.0.0.0"),
        "Opera UA suffix missing:\n{req}"
    );
    // Opera uses "Not:A-Brand";v="99" (dash/colon/99) — not the
    // Chrome "Not.A/Brand";v="8" form. This distinguishes real
    // Opera captures from Chrome captures.
    assert!(
        req.to_lowercase()
            .contains(r#"sec-ch-ua: "not:a-brand";v="99", "opera";v="131""#),
        "Opera sec-ch-ua placeholder form missing:\n{req}"
    );
    assert!(
        req.to_lowercase().contains(r#""chromium";v="147""#),
        "Opera Chromium anchor 147 missing:\n{req}"
    );
}

#[tokio::test]
async fn opera_146_overlay_emits_130() {
    // Hold Opera 130 / Chromium 146 byte-for-byte (vendor-doc EXTRAPOLATED).
    // Asserting the explicit pairing protects against an accidental
    // table edit collapsing every Chromium major to one Opera version.
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
            .contains(r#"sec-ch-ua: "not:a-brand";v="99", "opera";v="130", "chromium";v="146""#),
        "Opera 146 sec-ch-ua missing:\n{req}"
    );
}

#[tokio::test]
async fn opera_148_overlay_emits_132() {
    // Opera 132 / Chromium 148 — the anchor a recorded Opera build pins
    // to. Sourced from Opera's official desktop release blog (Opera 132 Stable
    // on Chromium 148.0.7778.97); sec-ch-ua shape reuses the verified 129
    // template. Pin the explicit pairing so a table edit can't collapse it.
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
            .contains(r#"sec-ch-ua: "not:a-brand";v="99", "opera";v="132", "chromium";v="148""#),
        "Opera 148 sec-ch-ua missing:\n{req}"
    );
}

#[tokio::test]
async fn opera_145_overlay_still_supported() {
    // Backward-compat: the original 129/145 live capture must keep
    // working after the anchor refresh. If this regresses, the
    // OPERA_PER_CHROMIUM table dropped its only live-captured row.
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
    // Vivaldi (per vivaldi.com/blog/technology/client-hints-or-client-lies)
    // deliberately omits its own brand from sec-ch-ua by default —
    // it ships ONLY Chromium + the GREASE placeholder. The UA does
    // carry a `Vivaldi/<build>` suffix though.
    let session = Session::vivaldi();
    let req = capture_navigate_headers(session).await;
    // Vivaldi 7.9 on Chromium 147 (vendor-doc EXTRAPOLATED).
    assert!(
        req.contains("Chrome/147.0.0.0 Safari/537.36 Vivaldi/7.9."),
        "Vivaldi UA suffix missing:\n{req}"
    );
    let lower = req.to_lowercase();
    // sec-ch-ua MUST drop "Google Chrome" and MUST NOT splice in "Vivaldi" —
    // catching either of those is what makes Vivaldi distinct from Chrome.
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
    // Vivaldi ships neither dnt nor sec-gpc by default (unlike Edge/Brave).
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
    // VIVALDI_BUILDS_PER_MAJOR currently only knows about Chromium 147.
    // 145/146 anchors must error rather than guess a build version.
    for bad in [144u32, 145, 146, 148] {
        let err = ChromiumBrand::Vivaldi
            .overlay(bad, Platform::Windows, "ua", r#""Google Chrome";v="147""#)
            .expect_err(&format!(
                "Vivaldi on Chromium {bad} must be rejected until captured"
            ));
        assert!(format!("{err}").contains("not verified"), "{err}");
    }
}

#[test]
fn vivaldi_overlay_on_mobile_errors() {
    let sch = r#""Google Chrome";v="147", "Not.A/Brand";v="8", "Chromium";v="147""#;
    for p in [Platform::Android, Platform::IOS] {
        let err = ChromiumBrand::Vivaldi
            .overlay(147, p, "ua", sch)
            .expect_err("mobile Vivaldi overlay must be rejected");
        assert!(format!("{err}").contains("not verified"));
    }
}

#[tokio::test]
async fn chrome_default_has_no_brand_overlay() {
    // Smoke check: stock Chrome session should NOT ship any of
    // the sibling-brand-specific headers. If it does, the
    // overlay's gate in `SessionBuilder::build` leaked.
    let session = Session::chrome();
    let req = capture_navigate_headers(session).await;
    assert!(!req.contains("Edg/"));
    assert!(!req.contains("OPR/"));
    assert!(!req.to_lowercase().contains("\r\ndnt: 1\r\n"));
    assert!(!req.to_lowercase().contains("\r\nsec-gpc: 1\r\n"));
    // The default browser is Chrome 150 (see Browser::default_browser);
    // its sec-ch-ua carries the "Google Chrome" brand somewhere in the value,
    // so match the brand anywhere rather than at the start. Intent: stock
    // Chrome identifies as Google Chrome and carries no sibling-brand token.
    let sec_ch_ua_line = req
        .lines()
        .find(|l| l.to_lowercase().starts_with("sec-ch-ua:"))
        .expect("sec-ch-ua header present")
        .to_lowercase();
    assert!(
        sec_ch_ua_line.contains(r#""google chrome";v="150""#),
        "Chrome sec-ch-ua should identify as Google Chrome 150:\n{req}"
    );
    assert!(
        !sec_ch_ua_line.contains("microsoft edge")
            && !sec_ch_ua_line.contains("opera")
            && !sec_ch_ua_line.contains("brave"),
        "stock Chrome sec-ch-ua must not carry a sibling-brand token:\n{req}"
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
    // 147-era template: hardcoding v="8" regardless of anchor makes
    // `.browser(Chrome145).brand(Edge)` ship a provably-wrong sec-ch-ua.
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
    // OPERA_PER_CHROMIUM covers Chromium 145..=149 today. A profile
    // anchored anywhere else (older 144 or some future 150+) still
    // has no published Opera version we can mimic — the build must
    // error rather than guess. We rely on `Browser::chromium_major`
    // returning a value outside the table, simulated here by hand-
    // calling the overlay directly so we don't need an unsupported
    // `Browser` variant in the public enum.
    let err = ChromiumBrand::Opera
        .overlay(150, Platform::Windows, "ua", "")
        .expect_err("Opera on Chrome 150 must be rejected until captured");
    assert!(format!("{err}").contains("not verified"));
}

#[tokio::test]
async fn user_dnt_override_wins_over_edge_default() {
    // A caller on an Edge session who explicitly sets dnt=0 MUST
    // NOT see both `dnt: 1` (brand default) and `dnt: 0` (user)
    // on the wire: shipping both lets some intermediaries coalesce
    // them to `dnt: 1, 0`.
    let session = Session::edge();
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
