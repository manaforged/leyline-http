use super::Protocol;
use crate::Session;
use crate::profile::Browser;
#[cfg(feature = "http3")]
use crate::profile::ChromiumBrand;

#[test]
fn rotate() {
    let session = Session::builder()
        .proxy("http://first:1")
        .build()
        .expect("bare session builds");
    let rotated = session
        .to_builder()
        .proxy("http://second:2")
        .build()
        .unwrap();

    let url = url::Url::parse("https://example.test/").unwrap();
    assert_eq!(
        rotated.proxy_for(&url, None).expect("valid proxy url"),
        Some("http://second:2"),
        "with_proxy must win over the build-time proxy"
    );
}

#[cfg(feature = "http3")]
#[test]
fn race() {
    let session = Session::builder()
        .identity(Browser::default_browser())
        .protocol(Protocol::Race)
        .build()
        .expect("chrome race session builds");
    assert_eq!(session.protocol_policy(), Protocol::Race);
}

#[cfg(feature = "http3")]
#[test]
fn chrome_race() {
    let session = Session::from(Browser::default());
    assert_eq!(session.protocol_policy(), Protocol::Race);
}

#[cfg(feature = "http3")]
#[test]
fn http1_then_edge_keeps_http1() {
    let session = Session::builder()
        .protocol(Protocol::Http1)
        .identity(
            crate::core::Identity::from(Browser::default_browser()).with_brand(ChromiumBrand::Edge),
        )
        .build()
        .expect("edge http1 session builds");
    assert_eq!(session.protocol_policy(), Protocol::Http1);
}

#[cfg(feature = "http3")]
#[test]
fn http1_then_chrome_keeps_http1() {
    let session = Session::builder()
        .protocol(Protocol::Http1)
        .identity(Browser::default_browser())
        .build()
        .expect("chrome http1 session builds");
    assert_eq!(session.protocol_policy(), Protocol::Http1);
}

#[test]
fn bare() {
    let session = Session::builder().build().unwrap();
    assert_eq!(session.protocol_policy(), Protocol::Auto);
}

#[cfg(feature = "http3")]
#[test]
fn brave_firefox_and_safari_race_http3() {
    for browser in [
        Browser::brave(),
        Browser::firefox(),
        Browser::safari(),
        Browser::safari_ios(),
    ] {
        let session = Session::builder().identity(browser).build().unwrap();
        assert_eq!(session.protocol_policy(), Protocol::Race, "{browser}");
    }
}
