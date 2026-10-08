use leyline::{Browser, Identity, Platform, Session};

#[test]
fn with_identity_switches_the_headers_with_the_hello() {
    let chrome = Session::builder()
        .browser(Browser::Chrome154)
        .platform(Platform::Windows)
        .build()
        .unwrap();
    let firefox = chrome
        .with_identity(Identity::locked(Browser::Firefox156, Platform::Windows))
        .unwrap();
    let agent = firefox.identity().user_agent().to_owned();
    assert!(agent.contains("Firefox/156"), "{agent}");
}

#[test]
fn the_last_browser_call_wins_over_an_earlier_identity() {
    let session = Session::builder()
        .identity(Identity::locked(Browser::Chrome148, Platform::Windows))
        .browser(Browser::Chrome154)
        .build()
        .unwrap();
    let agent = session.identity().user_agent().to_owned();
    assert!(agent.contains("Chrome/154"), "{agent}");
}

#[test]
fn a_session_user_agent_header_is_the_identity_user_agent() {
    let session = Session::builder()
        .browser(Browser::Chrome154)
        .platform(Platform::Windows)
        .headers([("user-agent", "probe/1.0")])
        .build()
        .unwrap();
    assert_eq!(session.identity().user_agent(), "probe/1.0");
    let switched = session
        .with_identity(Identity::locked(Browser::Firefox156, Platform::Windows))
        .unwrap();
    assert_eq!(switched.identity().user_agent(), "probe/1.0");
}

#[cfg(feature = "http3")]
#[test]
fn an_http3_session_refuses_an_identity_without_http3() {
    let session = Session::builder()
        .browser(Browser::Chrome154)
        .platform(Platform::Windows)
        .protocol(leyline::ProtocolPolicy::Http3)
        .build()
        .unwrap();
    let err = session
        .with_identity(Identity::locked(Browser::SafariIOS17, Platform::IOS))
        .unwrap_err();
    assert_eq!(err.kind(), leyline::Kind::Config, "{err:?}");
}
