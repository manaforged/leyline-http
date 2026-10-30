use leyline::{Browser, Platform, Session, SessionBuilder};

#[test]
fn with_identity_switches_the_headers_with_the_hello() {
    let chrome = Session::builder()
        .browser(Browser::Chrome154)
        .platform(Platform::Windows)
        .build()
        .unwrap();
    let firefox = chrome
        .to_builder()
        .browser(Browser::Firefox156)
        .platform(Platform::Windows)
        .build()
        .unwrap();
    let agent = firefox.user_agent().to_owned();
    assert!(agent.contains("Firefox/156"), "{agent}");
}

#[test]
fn the_last_browser_call_wins_over_an_earlier_identity() {
    let session = Session::builder()
        .browser(Browser::Chrome148)
        .platform(Platform::Windows)
        .browser(Browser::Chrome154)
        .build()
        .unwrap();
    let agent = session.user_agent().to_owned();
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
    assert_eq!(session.user_agent(), "probe/1.0");
    let switched = session
        .to_builder()
        .browser(Browser::Firefox156)
        .platform(Platform::Windows)
        .build()
        .unwrap();
    assert_eq!(switched.user_agent(), "probe/1.0");
}

#[cfg(feature = "http3")]
#[test]
fn an_http3_session_refuses_an_identity_without_http3() {
    let session = Session::builder()
        .browser(Browser::Chrome154)
        .platform(Platform::Windows)
        .protocol(leyline::Protocol::Http3)
        .build()
        .unwrap();
    let err = session
        .to_builder()
        .browser(Browser::SafariIOS17)
        .platform(Platform::IOS)
        .build()
        .err()
        .unwrap();
    assert_eq!(err.kind(), leyline::Kind::Config, "{err:?}");
}

#[test]
fn the_session_identity_rebuilds_the_same_session() {
    let session = Session::builder()
        .browser(Browser::Edge154)
        .platform(Platform::Windows)
        .build()
        .unwrap();
    let saved = serde_json::to_string(&session).unwrap();
    assert!(
        session.user_agent().contains(" Edg/154."),
        "{}",
        session.user_agent()
    );
    let rebuilt = serde_json::from_str::<SessionBuilder>(&saved)
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(rebuilt.profile_id(), session.profile_id());
}

#[tokio::test]
async fn every_session_identity_rebuilds_the_same_fingerprint() {
    let server = leyline::testing::Server::http(|_| leyline::testing::Response::new(200))
        .await
        .unwrap();
    let chrome = Browser::Chrome154;
    let toml = Session::from(chrome).profile().unwrap().to_owned();
    let builders = [
        Session::builder()
            .browser(Browser::Edge154)
            .platform(Platform::MacOS),
        Session::builder()
            .profile(&toml)
            .platform(Platform::Linux)
            .languages(["de-DE", "de"]),
        Session::builder().platform(Platform::Windows),
    ];
    for builder in builders {
        let first = builder.build().unwrap();
        let saved = serde_json::to_string(&first).unwrap();
        let again = serde_json::from_str::<SessionBuilder>(&saved)
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(again.profile_id(), first.profile_id());
        assert_eq!(
            (
                again.browser(),
                again.hello(),
                again.platform(),
                again.languages()
            ),
            (
                first.browser(),
                first.hello(),
                first.platform(),
                first.languages()
            )
        );
        let mut sent = Vec::new();
        for session in [&first, &again] {
            session.get(server.url("/")).send().await.unwrap();
            let request = server.next_request().await.unwrap();
            sent.push(request.headers);
        }
        assert_eq!(sent[0], sent[1]);
    }
}
