use super::*;

#[test]
fn anchor_names_match_chrome_slots() {
    assert_eq!(HeaderPosition::AfterCchUa.anchor_name(), "sec-ch-ua");
    assert_eq!(
        HeaderPosition::AfterCchUaPlatform.anchor_name(),
        "sec-ch-ua-platform"
    );
    assert_eq!(HeaderPosition::AfterUserAgent.anchor_name(), "user-agent");
    assert_eq!(
        HeaderPosition::BeforeAcceptEncoding.anchor_name(),
        "accept-encoding"
    );
}

#[test]
fn before_anchor_flagged() {
    assert!(HeaderPosition::BeforeCchUa.is_before());
    assert_eq!(HeaderPosition::BeforeCchUa.anchor_name(), "sec-ch-ua");
    assert!(HeaderPosition::BeforeAcceptEncoding.is_before());
    assert!(!HeaderPosition::AfterCchUa.is_before());
}

#[test]
fn infer_anchor_well_known_headers() {
    assert_eq!(
        infer_anchor("origin"),
        Some(HeaderPosition::AfterContentType)
    );
    assert_eq!(
        infer_anchor("Origin"),
        Some(HeaderPosition::AfterContentType)
    );
    assert_eq!(
        infer_anchor("authorization"),
        Some(HeaderPosition::AfterUserAgent)
    );
    assert_eq!(
        infer_anchor("X-Csrf-Token"),
        Some(HeaderPosition::AfterUserAgent)
    );
    assert_eq!(
        infer_anchor("x-requested-with"),
        Some(HeaderPosition::AfterUserAgent)
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
