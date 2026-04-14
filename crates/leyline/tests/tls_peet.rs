//! Integration tests against tls.peet.ws
//!
//! Offline tests (no network):
//!   cargo test -p leyline --test tls_peet
//!
//! Live tests (need transport + network, ignored by default):
//!   cargo test -p leyline --test tls_peet -- --ignored --nocapture

use leyline::{Browser, Platform};
use leyline_profile::{ALL_BROWSERS, PROFILE_COUNT};

// ─── Offline: profile data integrity ────────────────────────────────────

#[test]
fn profile_count_matches_constant() {
    let reg = leyline_profile::ProfileRegistry::builtin();
    assert_eq!(reg.len(), PROFILE_COUNT);
}

#[test]
fn every_browser_variant_has_profile() {
    let reg = leyline_profile::ProfileRegistry::builtin();
    for browser in ALL_BROWSERS {
        assert!(
            reg.get_browser(browser).is_some(),
            "no profile for {browser}"
        );
    }
}

#[test]
fn every_profile_has_fingerprint_expectation() {
    let reg = leyline_profile::ProfileRegistry::builtin();
    for browser in ALL_BROWSERS {
        let profile = reg.get_browser(browser).unwrap();
        let has_ja4 = profile.expected_ja4().is_some();
        let has_h2 = profile.expected_h2_fingerprint().is_some();
        assert!(
            has_ja4 || has_h2,
            "{browser} has no expected fingerprints in TOML"
        );
    }
}

#[test]
fn h2_fingerprints_match_toml_expectations() {
    let reg = leyline_profile::ProfileRegistry::builtin();
    let mut checked = 0;
    for browser in ALL_BROWSERS {
        let profile = reg.get_browser(browser).unwrap();
        if let Some(expected) = profile.expected_h2_fingerprint() {
            let h2 = leyline_h2::H2Config::from_profile(&profile.h2);
            let actual = h2.akamai_fingerprint();
            assert_eq!(actual, expected, "H2 mismatch for {browser}");
            checked += 1;
        }
    }
    assert!(checked >= 8, "expected at least 8 H2 fingerprints, got {checked}");
}

#[test]
fn session_builder_resolves_all_valid_combos() {
    let combos: Vec<(Browser, Platform)> = vec![
        (Browser::Chrome147, Platform::Windows),
        (Browser::Chrome147, Platform::MacOS),
        (Browser::Chrome147, Platform::Linux),
        (Browser::Chrome146, Platform::Windows),
        (Browser::Chrome145, Platform::Windows),
        (Browser::Firefox148, Platform::Windows),
        (Browser::Firefox148, Platform::Linux),
        (Browser::Safari18, Platform::MacOS),
        (Browser::OkHttpAndroid10, Platform::Android),
        (Browser::OkHttpAndroid7, Platform::Android),
        (Browser::SafariiOS15, Platform::IOS),
        (Browser::SafariiOS17, Platform::IOS),
        (Browser::SafariiOS18, Platform::IOS),
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
fn session_shortcuts_work() {
    let chrome = leyline::Session::chrome();
    assert!(chrome.is_ok());
    assert_eq!(chrome.unwrap().browser(), Browser::Chrome147);

    let firefox = leyline::Session::firefox();
    assert!(firefox.is_ok());
    assert_eq!(firefox.unwrap().browser(), Browser::Firefox148);

    let safari = leyline::Session::safari();
    assert!(safari.is_ok());
    assert_eq!(safari.unwrap().browser(), Browser::Safari18);
}

// ─── Live: TLS fingerprint verification ──────────────────────

#[test]
#[ignore = "needs transport"]
fn live_chrome147_tls_and_h2_fingerprint() {
    // GET https://tls.peet.ws/api/all
    // Assert: ja4 == profile.expected_ja4()
    // Assert: akamai H2 fingerprint == profile.expected_h2_fingerprint()
    todo!("wire transport");
}

#[test]
#[ignore = "needs transport"]
fn live_all_profiles_fingerprint_sweep() {
    // For each browser in ALL_BROWSERS:
    //   build session, GET tls.peet.ws, compare JA4 + H2
    todo!("wire transport");
}

// ─── Live: request behavior verification ─────────────────────
// These prove Session.execute() materializes requests correctly,
// not just that the TLS handshake looks right.

#[test]
#[ignore = "needs transport"]
fn live_navigate_sends_correct_headers() {
    // session.navigate(url) must send:
    // - sec-ch-ua, sec-ch-ua-mobile, sec-ch-ua-platform
    // - upgrade-insecure-requests: 1
    // - sec-fetch-site: none, sec-fetch-mode: navigate, sec-fetch-dest: document
    // - accept-encoding: gzip, deflate, br, zstd
    // Verify against httpbin or echo server.
    todo!("wire transport");
}

#[test]
#[ignore = "needs transport"]
fn live_post_json_sends_body_and_content_type() {
    // session.post_json(url, &data) must send:
    // - content-type: application/json
    // - body == serde_json::to_vec(&data)
    todo!("wire transport");
}

#[test]
#[ignore = "needs transport"]
fn live_cookies_persist_across_requests() {
    // 1. GET url that sets Set-Cookie
    // 2. GET same domain again
    // 3. Assert Cookie header is sent on second request
    todo!("wire transport");
}

#[test]
#[ignore = "needs transport"]
fn live_redirects_followed_and_chain_recorded() {
    // GET url that 302s → assert final URL != original
    // Assert redirect_chain is populated
    todo!("wire transport");
}
