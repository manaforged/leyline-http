use super::*;

#[test]
fn anchor_names_match_chrome_slots() {
    assert_eq!(HeaderAnchor::AfterCchUa.anchor_name(), "sec-ch-ua");
    assert_eq!(
        HeaderAnchor::AfterCchUaPlatform.anchor_name(),
        "sec-ch-ua-platform"
    );
    assert_eq!(HeaderAnchor::AfterUserAgent.anchor_name(), "user-agent");
    assert_eq!(
        HeaderAnchor::BeforeAcceptEncoding.anchor_name(),
        "accept-encoding"
    );
}

#[test]
fn before_anchor_flagged() {
    assert!(HeaderAnchor::BeforeAcceptEncoding.is_before());
    assert!(!HeaderAnchor::AfterCchUa.is_before());
}

#[test]
fn infer_anchor_well_known_headers() {
    assert_eq!(infer_anchor("origin"), Some(HeaderAnchor::AfterContentType));
    assert_eq!(infer_anchor("Origin"), Some(HeaderAnchor::AfterContentType));
    assert_eq!(
        infer_anchor("authorization"),
        Some(HeaderAnchor::AfterUserAgent)
    );
    assert_eq!(
        infer_anchor("X-Csrf-Token"),
        Some(HeaderAnchor::AfterUserAgent)
    );
    assert_eq!(
        infer_anchor("x-requested-with"),
        Some(HeaderAnchor::AfterUserAgent)
    );
}

#[test]
fn infer_anchor_none_for_custom_headers() {
    assert_eq!(infer_anchor("x-extra-6"), None);
    assert_eq!(infer_anchor("x-vendor-whatever"), None);
    assert_eq!(infer_anchor("x-custom"), None);
}

#[test]
fn infer_anchor_none_for_preset_owned_headers() {
    assert_eq!(infer_anchor("accept"), None);
    assert_eq!(infer_anchor("user-agent"), None);
    assert_eq!(infer_anchor("accept-language"), None);
}
