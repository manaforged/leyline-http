use super::*;

#[test]
fn chrome_brand_has_no_overlay() {
    let ua = "Mozilla/5.0 ...";
    assert!(
        ChromiumBrand::Chrome
            .overlay(147, Platform::Windows, ua)
            .unwrap()
            .is_none()
    );
}

#[test]
fn edge_overlay_populates_ua_and_headers() {
    let ua = "Mozilla/5.0 ... Chrome/147.0.0.0 Safari/537.36";
    let o = ChromiumBrand::Edge
        .overlay(147, Platform::Windows, ua)
        .unwrap()
        .unwrap();
    assert!(
        o.user_agent.ends_with(" Edg/147.0.0.0"),
        "Edge UA must end with reduced Edg/{{major}}.0.0.0: {}",
        o.user_agent
    );
    assert!(o.sec_ch_ua.contains(r#""Microsoft Edge";v="147""#));
}

#[test]
fn opera_overlay_matches_live_capture() {
    let ua = "Mozilla/5.0 ... Chrome/145.0.0.0 Safari/537.36";
    let o = ChromiumBrand::Opera
        .overlay(145, Platform::Windows, ua)
        .unwrap()
        .unwrap();
    assert!(o.user_agent.ends_with(" OPR/129.0.0.0"));
    assert_eq!(
        o.sec_ch_ua,
        r#""Not:A-Brand";v="99", "Opera";v="129", "Chromium";v="145""#
    );
}

#[test]
fn edge_overlay_on_desktop_ok_for_all_three_platforms() {
    for p in [Platform::Windows, Platform::MacOS, Platform::Linux] {
        ChromiumBrand::Edge
            .overlay(147, p, "ua")
            .expect("expected Ok");
    }
}

#[test]
fn edge_overlay_on_mobile_errors() {
    for p in [Platform::Android, Platform::IOS] {
        let err = ChromiumBrand::Edge.overlay(147, p, "ua").unwrap_err();
        assert!(matches!(err, BrandOverlayError::Unverified { .. }));
    }
}

#[test]
fn opera_overlay_only_accepts_verified_anchors() {
    for (chromium, expected_opera) in [
        (145u32, 129u32),
        (146, 130),
        (147, 131),
        (148, 132),
        (149, 133),
        (150, 134),
        (151, 135),
        (152, 136),
    ] {
        let o = ChromiumBrand::Opera
            .overlay(chromium, Platform::Windows, "ua")
            .unwrap()
            .unwrap();
        assert!(
            o.user_agent
                .ends_with(&format!(" OPR/{expected_opera}.0.0.0")),
            "Chromium {chromium} should map to OPR/{expected_opera}: {}",
            o.user_agent
        );
    }
    for bad in [144u32, 153] {
        ChromiumBrand::Opera
            .overlay(bad, Platform::Windows, "ua")
            .expect_err("expected Err");
    }
    for p in [Platform::Android, Platform::IOS] {
        ChromiumBrand::Opera
            .overlay(147, p, "ua")
            .expect_err("expected Err");
    }
}

#[test]
fn error_display_uses_platform_display_not_debug() {
    let err = BrandOverlayError::Unverified {
        brand: ChromiumBrand::Opera,
        chromium_major: 147,
        platform: Platform::MacOS,
    };
    let msg = format!("{err}");
    assert!(
        msg.contains("/ macOS /") || msg.contains("/ macOS"),
        "{msg}"
    );
    assert!(!msg.contains("MacOS"), "{msg}");
}
