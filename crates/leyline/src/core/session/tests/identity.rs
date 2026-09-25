use super::super::Session;
use crate::profile::{Browser, Platform};
use crate::{Identity, Kind};

#[test]
fn locked_keeps_http_and_tls_on_one_browser() {
    let id = Identity::locked(Browser::Chrome152, Platform::Windows);
    assert_eq!(id.http(), Browser::Chrome152);
    assert_eq!(id.tls(), Browser::Chrome153);
    assert_eq!(id.platform(), Platform::Windows);
}

#[test]
fn locked_collapses_tls_to_hello_owner() {
    let id = Identity::locked(Browser::Chrome148, Platform::Windows);
    assert_eq!(id.http(), Browser::Chrome148);
    assert_eq!(id.tls(), Browser::Chrome149);
    let id = Identity::locked(Browser::Chrome151, Platform::Windows);
    assert_eq!(id.http(), Browser::Chrome151);
    assert_eq!(id.tls(), Browser::Chrome151);
}

#[test]
fn rotate_tls_same_family_keeps_http() {
    let id = Identity::locked(Browser::Chrome150, Platform::Windows)
        .rotate_tls(Browser::Chrome146)
        .expect("chrome 150 → 146 is the same family");
    assert_eq!(id.http(), Browser::Chrome150);
    assert_eq!(id.tls(), Browser::Chrome149);
    assert_eq!(id.platform(), Platform::Windows);
}

#[test]
fn switch_family_chrome_to_firefox_keeps_platform() {
    let id = Identity::locked(Browser::Chrome150, Platform::Windows)
        .switch_family(Browser::Firefox152)
        .expect("chrome to firefox family switch");
    assert_eq!(id.http(), Browser::Firefox152);
    assert_eq!(id.tls(), Browser::Firefox152);
    assert_eq!(id.platform(), Platform::Windows);
    Session::builder()
        .identity(id)
        .build()
        .expect("firefox identity builds");
}

#[test]
fn switch_family_safari_macos_to_chrome_and_firefox() {
    let safari = Identity::locked(Browser::Safari18, Platform::MacOS);
    let chrome = safari
        .switch_family(Browser::Chrome150)
        .expect("safari → chrome");
    let firefox = safari
        .switch_family(Browser::Firefox152)
        .expect("safari → firefox");
    assert_eq!(chrome.http(), Browser::Chrome150);
    assert_eq!(chrome.platform(), Platform::MacOS);
    assert_eq!(firefox.http(), Browser::Firefox152);
    Session::builder()
        .identity(chrome)
        .build()
        .expect("safari→chrome builds");
    Session::builder()
        .identity(firefox)
        .build()
        .expect("safari→firefox builds");
}

#[test]
fn switch_family_rejects_same_family() {
    let err = Identity::locked(Browser::Chrome150, Platform::Windows)
        .switch_family(Browser::Chrome146)
        .expect_err("a switch needs another family");
    assert_eq!(err.kind(), Kind::Config, "expected Config, got {err}");
    let message = err.message().expect("config errors carry a message");
    assert!(
        message.contains("rotate_tls"),
        "unexpected config: {message}"
    );
}

#[test]
fn switch_family_rejects_safari_on_windows() {
    let err = Identity::locked(Browser::Chrome150, Platform::Windows)
        .switch_family(Browser::Safari18)
        .expect_err("safari has no windows identity");
    assert_eq!(err.kind(), Kind::Config, "expected Config, got {err}");
}

#[test]
fn rotate_tls_rejects_other_family() {
    let err = Identity::locked(Browser::Chrome150, Platform::Windows)
        .rotate_tls(Browser::Firefox152)
        .expect_err("chrome → firefox is not a legal rotate");
    assert_eq!(err.kind(), Kind::Config, "expected Config, got {err}");
    let message = err.message().expect("config errors carry a message");
    assert!(
        message.contains("not the same family"),
        "unexpected config: {message}"
    );
}

#[test]
fn identity_apply_locked_builds() {
    let id = Identity::locked(Browser::Chrome150, Platform::Windows);
    Session::builder()
        .identity(id)
        .build()
        .expect("locked chrome 150 windows builds");
}

#[test]
fn older_public_hellos_build() {
    for browser in [Browser::Chrome145, Browser::Firefox148] {
        let id = match browser {
            Browser::Chrome145 => Identity::locked(Browser::Chrome150, Platform::Windows)
                .rotate_tls(browser)
                .expect("same family"),
            Browser::Firefox148 => Identity::locked(Browser::Firefox152, Platform::Windows)
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
    let id = Identity::locked(Browser::Chrome150, Platform::Windows)
        .rotate_tls(Browser::Chrome148)
        .expect("148 is 149's hello");
    assert_eq!(id.tls(), Browser::Chrome149);
}

#[test]
fn identity_apply_rotated_tls_builds() {
    let id = Identity::locked(Browser::Chrome150, Platform::Windows)
        .rotate_tls(Browser::Chrome146)
        .expect("rotate");
    Session::builder()
        .identity(id)
        .build()
        .expect("chrome 150 HTTP + chrome 146 TLS builds");
}
