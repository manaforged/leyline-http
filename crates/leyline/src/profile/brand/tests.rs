use super::*;

#[test]
fn swap_brand_preserves_chrome147_grease_form() {
    let chrome = r#""Google Chrome";v="147", "Not.A/Brand";v="8", "Chromium";v="147""#;
    assert_eq!(
        swap_brand(chrome, "Microsoft Edge"),
        r#""Microsoft Edge";v="147", "Not.A/Brand";v="8", "Chromium";v="147""#
    );
}

#[test]
fn swap_brand_preserves_chrome145_grease_form() {
    let chrome = r#""Google Chrome";v="145", "Not_A Brand";v="24", "Chromium";v="145""#;
    assert_eq!(
        swap_brand(chrome, "Microsoft Edge"),
        r#""Microsoft Edge";v="145", "Not_A Brand";v="24", "Chromium";v="145""#
    );
}

#[test]
fn drop_brand_removes_chrome_entry_only() {
    let chrome = r#""Google Chrome";v="147", "Not.A/Brand";v="8", "Chromium";v="147""#;
    assert_eq!(
        drop_brand(chrome, "Google Chrome"),
        r#""Not.A/Brand";v="8", "Chromium";v="147""#
    );
}

#[test]
fn drop_brand_preserves_remaining_slot_order() {
    let chrome = r#""Not_A Brand";v="24", "Chromium";v="146", "Google Chrome";v="146""#;
    assert_eq!(
        drop_brand(chrome, "Google Chrome"),
        r#""Not_A Brand";v="24", "Chromium";v="146""#
    );
}

#[test]
fn swap_brand_preserves_chrome146_slot_order() {
    let chrome = r#""Google Chrome";v="146", "Chromium";v="146", "Not_A Brand";v="24""#;
    assert_eq!(
        swap_brand(chrome, "Brave"),
        r#""Brave";v="146", "Chromium";v="146", "Not_A Brand";v="24""#
    );
}

#[test]
fn chrome_brand_has_no_overlay() {
    let ua = "Mozilla/5.0 ...";
    let sch = r#""Google Chrome";v="147", "Not.A/Brand";v="8", "Chromium";v="147""#;
    assert!(
        ChromiumBrand::Chrome
            .overlay(147, Platform::Windows, ua, sch)
            .unwrap()
            .is_none()
    );
}

#[test]
fn edge_overlay_populates_ua_and_headers() {
    let ua = "Mozilla/5.0 ... Chrome/147.0.0.0 Safari/537.36";
    let sch = r#""Google Chrome";v="147", "Not.A/Brand";v="8", "Chromium";v="147""#;
    let o = ChromiumBrand::Edge
        .overlay(147, Platform::Windows, ua, sch)
        .unwrap()
        .unwrap();
    assert!(
        o.user_agent.ends_with(" Edg/147.0.0.0"),
        "Edge UA must end with reduced Edg/{{major}}.0.0.0: {}",
        o.user_agent
    );
    assert!(o.sec_ch_ua.contains(r#""Microsoft Edge";v="147""#));
    assert_eq!(o.extra_headers, vec![("dnt".into(), "1".into())]);
    assert!(o.navigate_accept.is_none());
}

#[test]
fn opera_overlay_matches_live_capture() {
    let ua = "Mozilla/5.0 ... Chrome/145.0.0.0 Safari/537.36";
    let o = ChromiumBrand::Opera
        .overlay(145, Platform::Windows, ua, "")
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
    let sch = r#""Google Chrome";v="147", "Not.A/Brand";v="8", "Chromium";v="147""#;
    for p in [Platform::Windows, Platform::MacOS, Platform::Linux] {
        assert!(ChromiumBrand::Edge.overlay(147, p, "ua", sch).is_ok());
    }
}

#[test]
fn edge_overlay_on_mobile_errors() {
    let sch = r#""Google Chrome";v="147", "Not.A/Brand";v="8", "Chromium";v="147""#;
    for p in [Platform::Android, Platform::IOS] {
        let err = ChromiumBrand::Edge.overlay(147, p, "ua", sch).unwrap_err();
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
            .overlay(chromium, Platform::Windows, "ua", "")
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
        assert!(
            ChromiumBrand::Opera
                .overlay(bad, Platform::Windows, "ua", "")
                .is_err()
        );
    }
    for p in [Platform::Android, Platform::IOS] {
        assert!(ChromiumBrand::Opera.overlay(147, p, "ua", "").is_err());
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
