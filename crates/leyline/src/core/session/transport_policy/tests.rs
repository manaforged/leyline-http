use super::ProtocolPolicy;
use crate::Session;

#[test]
fn rotate() {
    let session = Session::builder()
        .proxy("http://first:1")
        .build()
        .expect("bare session builds");
    let rotated = session.with_proxy("http://second:2");

    let url = url::Url::parse("https://example.test/").unwrap();
    assert_eq!(
        rotated.effective_proxy_for(&url, None),
        Some("http://second:2"),
        "with_proxy must win over the build-time proxy"
    );
}

#[cfg(feature = "http3")]
#[test]
fn race() {
    let session = Session::builder()
        .chrome()
        .race()
        .build()
        .expect("chrome race session builds");
    assert_eq!(session.protocol_policy(), ProtocolPolicy::Race);
}

#[cfg(feature = "http3")]
#[test]
fn chrome_race() {
    let session = Session::chrome();
    assert_eq!(session.protocol_policy(), ProtocolPolicy::Race);
}

#[cfg(feature = "http3")]
#[test]
fn edge_race() {
    let session = Session::edge();
    assert_eq!(session.protocol_policy(), ProtocolPolicy::Race);
}

#[cfg(feature = "http3")]
#[test]
fn opera_race() {
    let session = Session::opera();
    assert_eq!(session.protocol_policy(), ProtocolPolicy::Race);
}

#[cfg(feature = "http3")]
#[test]
fn brave_race() {
    let session = Session::brave();
    assert_eq!(session.protocol_policy(), ProtocolPolicy::Race);
}

#[cfg(feature = "http3")]
#[test]
fn vivaldi_race() {
    let session = Session::vivaldi();
    assert_eq!(session.protocol_policy(), ProtocolPolicy::Race);
}

#[cfg(feature = "http3")]
#[test]
fn http1_then_edge_keeps_http1() {
    let session = Session::builder()
        .http1()
        .edge()
        .build()
        .expect("edge http1 session builds");
    assert_eq!(session.protocol_policy(), ProtocolPolicy::Http1);
}

#[cfg(feature = "http3")]
#[test]
fn http1_then_chrome_keeps_http1() {
    let session = Session::builder()
        .http1()
        .chrome()
        .build()
        .expect("chrome http1 session builds");
    assert_eq!(session.protocol_policy(), ProtocolPolicy::Http1);
}

#[test]
fn bare() {
    let session = Session::new();
    assert_eq!(session.protocol_policy(), ProtocolPolicy::Auto);
}

#[test]
fn firefox_auto() {
    let session = Session::firefox();
    assert_eq!(session.protocol_policy(), ProtocolPolicy::Auto);
}
