use super::super::Session;
use crate::profile::{Browser, Platform};
use crate::{Error, Identity};

#[test]
fn locked_keeps_http_and_tls_on_one_browser() {
    let id = Identity::locked(Browser::Chrome150, Platform::Windows);
    assert_eq!(id.http(), Browser::Chrome150);
    assert_eq!(id.tls(), Browser::Chrome150);
    assert_eq!(id.platform(), Platform::Windows);
}

#[test]
fn locked_collapses_tls_to_hello_owner() {
    let id = Identity::locked(Browser::Chrome148, Platform::Windows);
    assert_eq!(id.http(), Browser::Chrome148);
    assert_eq!(id.tls(), Browser::Chrome147);
}

#[test]
fn rotate_tls_same_family_keeps_http() {
    let id = Identity::locked(Browser::Chrome150, Platform::Windows)
        .rotate_tls(Browser::Chrome146)
        .expect("chrome 150 → 146 is the same family");
    assert_eq!(id.http(), Browser::Chrome150);
    assert_eq!(id.tls(), Browser::Chrome146);
    assert_eq!(id.platform(), Platform::Windows);
}

#[test]
fn switch_family_chrome_to_firefox_keeps_platform() {
    let id = Identity::locked(Browser::Chrome150, Platform::Windows)
        .pass(Browser::Firefox152)
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
    let chrome = safari.pass(Browser::Chrome150).expect("safari → chrome");
    let firefox = safari.pass(Browser::Firefox152).expect("safari → firefox");
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
        .pass(Browser::Chrome146)
        .expect_err("a switch needs another family");
    match err {
        Error::Config(message) => {
            assert!(
                message.contains("rotate_tls"),
                "unexpected config: {message}"
            );
        }
        other => panic!("expected Config, got {other}"),
    }
}

#[test]
fn switch_family_rejects_safari_on_windows() {
    let err = Identity::locked(Browser::Chrome150, Platform::Windows)
        .pass(Browser::Safari18)
        .expect_err("safari has no windows identity");
    match err {
        Error::Config(_) => {}
        other => panic!("expected Config, got {other}"),
    }
}

#[test]
fn pass_library_windows_is_firefox() {
    let lib = Identity::locked(Browser::Chrome150, Platform::Windows).pass_library();
    let https: Vec<Browser> = lib.iter().map(|id| id.http()).collect();
    assert_eq!(https, vec![Browser::Firefox152]);
}

#[test]
fn pass_library_macos_is_firefox_and_safari() {
    let from_chrome = Identity::locked(Browser::Chrome150, Platform::MacOS).pass_library();
    let https: Vec<Browser> = from_chrome.iter().map(|id| id.http()).collect();
    assert_eq!(https, vec![Browser::Firefox152, Browser::Safari18]);
}

#[test]
fn rotate_tls_rejects_other_family() {
    let err = Identity::locked(Browser::Chrome150, Platform::Windows)
        .rotate_tls(Browser::Firefox152)
        .expect_err("chrome → firefox is not a legal rotate");
    match err {
        Error::Config(message) => {
            assert!(
                message.contains("not the same family"),
                "unexpected config: {message}"
            );
        }
        other => panic!("expected Config, got {other}"),
    }
}

#[test]
fn build_rejects_http_family_mismatch() {
    let err = Session::builder()
        .browser(Browser::Firefox152)
        .http_identity(Browser::Chrome150)
        .platform(Platform::Windows)
        .build()
        .expect_err("chrome HTTP on firefox TLS must fail");
    match err {
        Error::Config(message) => {
            assert!(
                message.contains("not the same family"),
                "unexpected config: {message}"
            );
        }
        other => panic!("expected Config, got {other}"),
    }
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
fn hello_library_is_three_chrome_hellos() {
    let lib = Identity::locked(Browser::Chrome150, Platform::Windows)
        .hello_library()
        .expect("chrome hello library");
    let tls: Vec<Browser> = lib.iter().map(|id| id.tls()).collect();
    assert_eq!(
        tls,
        vec![Browser::Chrome150, Browser::Chrome147, Browser::Chrome146,]
    );
    assert!(lib.iter().all(|id| id.http() == Browser::Chrome150));
    assert!(lib.iter().all(|id| id.platform() == Platform::Windows));
}

#[test]
fn rotate_hello_walks_distinct_chrome_ja4s() {
    let a = Identity::locked(Browser::Chrome150, Platform::Windows);
    let b = a.rotate_hello().expect("150 → 147");
    let c = b.rotate_hello().expect("147 → 146");
    let d = c.rotate_hello().expect("146 → 150");
    assert_eq!(b.tls(), Browser::Chrome147);
    assert_eq!(c.tls(), Browser::Chrome146);
    assert_eq!(d.tls(), Browser::Chrome150);
    assert_eq!(d.http(), Browser::Chrome150);
}

#[test]
fn rotate_tls_collapses_same_hello() {
    let id = Identity::locked(Browser::Chrome150, Platform::Windows)
        .rotate_tls(Browser::Chrome148)
        .expect("148 is 147's hello");
    assert_eq!(id.tls(), Browser::Chrome147);
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

#[test]
fn hello_library_stays_consistent() {
    let mint = Identity::locked(Browser::Chrome150, Platform::Windows);
    let lib = mint.hello_library().expect("chrome hello library");
    assert_eq!(
        mint.user_agent().expect("ua"),
        lib[0].user_agent().expect("ua")
    );
    for id in &lib {
        assert_eq!(id.http(), Browser::Chrome150);
        assert_eq!(id.platform(), Platform::Windows);
        assert_eq!(id.tls(), id.tls().hello_rep());
        assert_eq!(id.http().family(), id.tls().family());
        assert!(
            id.user_agent().expect("ua").contains("Chrome/150.0.0.0"),
            "HTTP UA must stay 150 for {:?}: {}",
            id.tls(),
            id.user_agent().expect("ua")
        );
        let session = Session::builder()
            .identity(*id)
            .build()
            .unwrap_or_else(|e| panic!("{} builds: {e}", id.tls()));
        assert_eq!(session.identity(), Some(*id));
        assert_eq!(session.browser(), Some(id.tls()));
    }
}

#[test]
fn firefox_hello_library_stays_consistent() {
    let lib = Identity::locked(Browser::Firefox152, Platform::Windows)
        .hello_library()
        .expect("firefox hello library");
    assert_eq!(
        lib.iter().map(|id| id.tls()).collect::<Vec<_>>(),
        vec![Browser::Firefox152, Browser::Firefox150]
    );
    for id in lib {
        assert_eq!(id.http(), Browser::Firefox152);
        assert!(
            id.user_agent().expect("ua").contains("Firefox/152.0"),
            "HTTP UA must stay 152 for {:?}: {}",
            id.tls(),
            id.user_agent().expect("ua")
        );
    }
}
