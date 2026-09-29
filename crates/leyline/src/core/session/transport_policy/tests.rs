use super::ProtocolPolicy;
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
    let rotated = session.with_proxy("http://second:2");

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
        .browser(Browser::default_browser())
        .protocol(ProtocolPolicy::Race)
        .build()
        .expect("chrome race session builds");
    assert_eq!(session.protocol_policy(), ProtocolPolicy::Race);
}

#[cfg(feature = "http3")]
#[test]
fn chrome_race() {
    let session = Session::new();
    assert_eq!(session.protocol_policy(), ProtocolPolicy::Race);
}

#[cfg(feature = "http3")]
#[test]
fn http1_then_edge_keeps_http1() {
    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .browser(Browser::default_browser())
        .brand(ChromiumBrand::Edge)
        .build()
        .expect("edge http1 session builds");
    assert_eq!(session.protocol_policy(), ProtocolPolicy::Http1);
}

#[cfg(feature = "http3")]
#[test]
fn http1_then_chrome_keeps_http1() {
    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .browser(Browser::default_browser())
        .build()
        .expect("chrome http1 session builds");
    assert_eq!(session.protocol_policy(), ProtocolPolicy::Http1);
}

#[test]
fn bare() {
    let session = Session::builder().build().unwrap();
    assert_eq!(session.protocol_policy(), ProtocolPolicy::Auto);
}

#[test]
fn firefox_auto() {
    let session = Session::builder()
        .browser(Browser::latest(crate::profile::Family::Firefox))
        .build()
        .unwrap();
    assert_eq!(session.protocol_policy(), ProtocolPolicy::Auto);
}
