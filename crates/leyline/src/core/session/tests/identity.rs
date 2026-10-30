use super::super::Session;
use crate::Cause;
use crate::core::Identity;
use crate::profile::{Browser, Platform};

#[test]
fn new_keeps_the_browser_on_both_sides() {
    let id = Identity::new(Browser::Chrome148, Platform::Windows);
    assert_eq!(id.http(), Some(Browser::Chrome148));
    assert_eq!(id.tls(), Some(Browser::Chrome148));
    assert_eq!(id.platform(), Platform::Windows);
}

#[test]
fn from_a_browser_claims_its_native_platform() {
    assert_eq!(
        Identity::from(Browser::Chrome148).platform(),
        Platform::Windows
    );
    assert_eq!(
        Identity::from(Browser::Safari26).platform(),
        Platform::MacOS
    );
    assert_eq!(
        Identity::from(Browser::SafariIOS27).platform(),
        Platform::IOS
    );
}

#[test]
fn rotate_tls_same_family_keeps_http() {
    let id = Identity::new(Browser::Chrome150, Platform::Windows)
        .rotate_tls(Browser::Chrome146)
        .expect("chrome 150 → 146 is the same family");
    assert_eq!(id.http(), Some(Browser::Chrome150));
    assert_eq!(id.tls(), Some(Browser::Chrome149));
    assert_eq!(id.platform(), Platform::Windows);
}

#[test]
fn an_identity_with_no_profile_for_the_platform_fails_at_build() {
    let err = Session::builder()
        .identity(Identity::new(Browser::Safari18, Platform::Windows))
        .build()
        .expect_err("safari has no windows identity");
    assert_eq!(err.cause(), Cause::Config, "expected Config, got {err}");
}

#[test]
fn rotate_tls_rejects_other_family() {
    let err = Identity::new(Browser::Chrome150, Platform::Windows)
        .rotate_tls(Browser::Firefox152)
        .expect_err("chrome → firefox is not a legal rotate");
    assert_eq!(err.cause(), Cause::Config, "expected Config, got {err}");
    let message = err.message().expect("config errors carry a message");
    assert!(
        message.contains("not the same family"),
        "unexpected config: {message}"
    );
}

#[test]
fn identity_apply_locked_builds() {
    let id = Identity::new(Browser::Chrome150, Platform::Windows);
    Session::builder()
        .identity(id)
        .build()
        .expect("locked chrome 150 windows builds");
}

#[test]
fn older_public_hellos_build() {
    for browser in [Browser::Chrome145, Browser::Firefox148] {
        let id = match browser {
            Browser::Chrome145 => Identity::new(Browser::Chrome150, Platform::Windows)
                .rotate_tls(browser)
                .expect("same family"),
            Browser::Firefox148 => Identity::new(Browser::Firefox152, Platform::Windows)
                .rotate_tls(browser)
                .expect("same family"),
            other => panic!("unexpected {other:?}"),
        };
        Session::builder()
            .identity(id)
            .build()
            .unwrap_or_else(|e| panic!("{browser} hello builds: {e}"));
    }
}

#[test]
fn rotate_tls_collapses_same_hello() {
    let id = Identity::new(Browser::Chrome150, Platform::Windows)
        .rotate_tls(Browser::Chrome148)
        .expect("148 is 149's hello");
    assert_eq!(id.tls(), Some(Browser::Chrome149));
}

#[test]
fn identity_apply_rotated_tls_builds() {
    let id = Identity::new(Browser::Chrome150, Platform::Windows)
        .rotate_tls(Browser::Chrome146)
        .expect("rotate");
    Session::builder()
        .identity(id)
        .build()
        .expect("chrome 150 HTTP + chrome 146 TLS builds");
}

#[test]
fn a_saved_identity_keeps_its_json() {
    let saved = include_str!("../../../../tests/fixtures/identity-58f586f6.json");
    let identity: Identity = serde_json::from_str(saved).unwrap();
    assert_eq!(serde_json::to_string(&identity).unwrap(), saved.trim());
}
