use leyline::{Browser, Platform};

#[test]
fn every_browser_variant_has_profile() {
    let reg = leyline::profile::ProfileRegistry::builtin();
    for browser in Browser::all().iter().copied() {
        assert!(
            reg.get_browser(browser).is_some(),
            "no profile for {browser}"
        );
    }
}

#[test]
fn every_profile_has_fingerprint_expectation() {
    let reg = leyline::profile::ProfileRegistry::builtin();
    for browser in Browser::all().iter().copied() {
        let profile = reg.get_browser(browser).unwrap();
        let has_ja4 = profile
            .tls
            .fingerprint
            .as_ref()
            .and_then(|f| f.ja4.as_deref())
            .is_some();
        let has_h2 = profile
            .h2
            .fingerprint
            .as_ref()
            .and_then(|f| f.akamai.as_deref())
            .is_some();
        assert!(
            has_ja4 || has_h2,
            "{browser} has no expected fingerprints in TOML"
        );
    }
}

#[test]
fn h2_fingerprints_match_toml_expectations() {
    let reg = leyline::profile::ProfileRegistry::builtin();
    let mut checked = 0;
    for browser in Browser::all().iter().copied() {
        let profile = reg.get_browser(browser).unwrap();
        if let Some(expected) = profile
            .h2
            .fingerprint
            .as_ref()
            .and_then(|f| f.akamai.as_deref())
        {
            let h2 = leyline::h2::H2Config::from_profile(&profile.h2).unwrap();
            let actual = h2.akamai_fingerprint();
            assert_eq!(actual, expected, "H2 mismatch for {browser}");
            checked += 1;
        }
    }
    assert!(
        checked >= 10,
        "expected all 10 H2 fingerprints to be checked, got {checked}"
    );
}

#[test]
fn h2_per_platform_overrides_resolve() {
    let reg = leyline::profile::ProfileRegistry::builtin();
    for browser in [Browser::Chrome145, Browser::Chrome146, Browser::Chrome147] {
        let profile = reg.get_browser(browser).unwrap();
        let resolved = profile.h2.resolve_for_platform(Platform::MacOS).unwrap();
        let h2 = leyline::h2::H2Config::from_profile(&resolved).unwrap();
        let actual = h2.akamai_fingerprint();
        let expected = profile
            .h2
            .platforms
            .get("macos")
            .and_then(|over| over.fingerprint.as_ref())
            .and_then(|fp| fp.akamai.as_deref())
            .or_else(|| {
                profile
                    .h2
                    .fingerprint
                    .as_ref()
                    .and_then(|f| f.akamai.as_deref())
            })
            .unwrap_or_else(|| panic!("{browser} has no macos H2 fingerprint expectation"));
        assert_eq!(actual, expected, "{browser} macOS H2 fingerprint mismatch");
        assert!(
            !actual.contains(";8:1"),
            "{browser} macOS Akamai must omit setting 8: {actual}"
        );
    }
}

#[test]
fn session_builder_resolves_all_valid_combos() {
    let combos: Vec<(Browser, Platform)> = vec![
        (Browser::Chrome147, Platform::Windows),
        (Browser::Chrome147, Platform::MacOS),
        (Browser::Chrome147, Platform::Linux),
        (Browser::Chrome147, Platform::Android),
        (Browser::Chrome148, Platform::Windows),
        (Browser::Chrome148, Platform::MacOS),
        (Browser::Chrome148, Platform::Linux),
        (Browser::Chrome148, Platform::Android),
        (Browser::Chrome149, Platform::Windows),
        (Browser::Chrome149, Platform::MacOS),
        (Browser::Chrome149, Platform::Linux),
        (Browser::Chrome149, Platform::Android),
        (Browser::Chrome150, Platform::Windows),
        (Browser::Chrome150, Platform::MacOS),
        (Browser::Chrome150, Platform::Linux),
        (Browser::Chrome150, Platform::Android),
        (Browser::Chrome146, Platform::Windows),
        (Browser::Chrome146, Platform::Android),
        (Browser::Chrome145, Platform::Windows),
        (Browser::Chrome145, Platform::Android),
        (Browser::Brave146, Platform::MacOS),
        (Browser::Firefox148, Platform::Windows),
        (Browser::Firefox148, Platform::Linux),
        (Browser::Firefox148, Platform::Android),
        (Browser::Firefox150, Platform::Windows),
        (Browser::Firefox150, Platform::Linux),
        (Browser::Firefox150, Platform::Android),
        (Browser::Firefox151, Platform::Windows),
        (Browser::Firefox151, Platform::Linux),
        (Browser::Firefox151, Platform::Android),
        (Browser::Firefox152, Platform::Windows),
        (Browser::Firefox152, Platform::Linux),
        (Browser::Firefox152, Platform::Android),
        (Browser::Safari18, Platform::MacOS),
        (Browser::OkHttpAndroid10, Platform::Android),
        (Browser::SafariIOS17, Platform::IOS),
        (Browser::SafariIOS18, Platform::IOS),
        (Browser::CfnetworkIOS18, Platform::IOS),
        (Browser::CfnetworkMacOS26, Platform::MacOS),
    ];
    for (browser, platform) in combos {
        let result = leyline::Session::builder()
            .browser(browser)
            .platform(platform)
            .build();
        assert!(result.is_ok(), "failed to build {browser} on {platform}");
    }
}

#[test]
fn crate_root_context_helpers_work() {
    let tls = leyline::TlsContext::from_profile(
        Browser::Chrome147.profile(),
        leyline::TlsMinVersion::Tls12,
    );
    assert!(tls.is_ok(), "tls context failed: {:?}", tls.err());

    #[cfg(feature = "http3")]
    {
        let quic = leyline::TlsContext::from_profile(
            Browser::Chrome147.profile(),
            leyline::TlsMinVersion::Tls13,
        );
        assert!(quic.is_ok(), "quic context failed: {:?}", quic.err());
    }

    for browser in Browser::all().iter().copied() {
        assert!(
            leyline::TlsContext::from_profile(browser.profile(), leyline::TlsMinVersion::Tls12)
                .is_ok(),
            "tls context failed for {browser}"
        );
    }
}

#[test]
fn profile_helper_returns_builtin() {
    let p = Browser::Chrome147.profile();
    assert_eq!(p.meta.browser, "chrome");
    assert_eq!(p.meta.version, 147);
    assert!(!p.tls.ciphers.is_empty());
}

#[test]
fn session_builder_rejects_http3_with_proxy_at_build_time() {
    let err = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .protocol(leyline::Protocol::Http3)
        .proxy("http://127.0.0.1:8080")
        .build()
        .expect_err("should reject http3+proxy at build");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("http/3") || msg.contains("http3") || msg.contains("proxy"),
        "error message should mention http3/proxy: {msg}"
    );
}

#[cfg(feature = "http3")]
#[tokio::test]
async fn http3_session_rejects_per_request_proxy_at_send_time() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .protocol(leyline::Protocol::Http3)
        .build()
        .expect("http3 session without proxy builds");
    let err = session
        .request(http::Method::GET, "https://example.com/")
        .proxy("http://127.0.0.1:9")
        .send()
        .await
        .expect_err("h3 + per-request proxy must refuse, not dial direct");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("http/3") || msg.contains("http3") || msg.contains("prox"),
        "error should mention http3/proxy, got: {msg}"
    );
}

#[cfg(feature = "http3")]
#[tokio::test]
async fn race_proxy() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .protocol(leyline::Protocol::Race)
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .expect("race session builds");
    let result = session
        .request(http::Method::GET, "https://example.com/")
        .proxy("http://127.0.0.1:9")
        .send()
        .await;
    assert!(
        result.is_err(),
        "request through a dead proxy must fail; success means the H3 race \
         leg bypassed the proxy and dialed direct"
    );
}

#[tokio::test]
async fn request_builder_timeout_overrides_session_default() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap();
    let start = std::time::Instant::now();
    let err = session
        .request(http::Method::GET, "https://192.0.2.1/")
        .timeout(std::time::Duration::from_millis(50))
        .send()
        .await
        .expect_err("should time out on unroutable address");
    let elapsed = start.elapsed();
    assert!(
        err.kind() == leyline::Kind::Timeout,
        "expected Error::new(leyline::Kind::Timeout), got {err:?}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "per-request timeout didn't fire — elapsed {elapsed:?}"
    );
}

#[test]
fn session_shortcuts_work() {
    assert_eq!(
        format!("{}", leyline::Session::from(Browser::default())),
        format!(
            "Session({}, {}, proxy=none)",
            Browser::default(),
            Platform::Windows
        )
    );
    let firefox = leyline::Session::builder()
        .browser(Browser::firefox())
        .platform(Platform::Windows)
        .build()
        .unwrap();
    assert_eq!(
        format!("{firefox}"),
        format!(
            "Session({}, {}, proxy=none)",
            Browser::firefox(),
            Platform::Windows
        )
    );
    let safari = leyline::Session::builder()
        .browser(Browser::Safari26)
        .platform(Platform::MacOS)
        .build()
        .unwrap();
    assert_eq!(
        format!("{safari}"),
        format!(
            "Session({}, {}, proxy=none)",
            Browser::Safari26,
            Platform::MacOS
        )
    );
}
