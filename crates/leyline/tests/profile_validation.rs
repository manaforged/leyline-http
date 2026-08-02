//! Negative-path validation for browser profiles.
//!
//! A fingerprinting library must fail loudly when profile data is malformed:
//! a silently-substituted default ships a wrong wire fingerprint, which CDN/WAF
//! edges soft-block (a profile-drift soft-block incident). Each test mutates
//! one field of a real built-in profile and asserts `H2Config::from_profile`
//! rejects it instead of degrading silently. The positive guard proves the
//! stricter validation does not reject any shipping profile.

use leyline::h2::H2Config;
use leyline::profile::{ALL_BROWSERS, BrowserProfile, H2Profile, ProfileRegistry};
use leyline::{Browser, Platform, TlsContext, TlsMinVersion};

const ALL_PLATFORMS: [Platform; 5] = [
    Platform::Windows,
    Platform::MacOS,
    Platform::Linux,
    Platform::Android,
    Platform::IOS,
];

/// A real, valid base profile to mutate one field at a time.
fn chrome_h2() -> H2Profile {
    chrome_profile().h2
}

/// An owned clone of a real built-in profile, for one-field mutation tests.
fn chrome_profile() -> BrowserProfile {
    ProfileRegistry::builtin()
        .get_browser(Browser::Chrome147)
        .expect("chrome147 built-in profile")
        .clone()
}

// ── C-1: an unknown settings_order key is a typo, not a no-op ────────────────
#[test]
fn unknown_settings_order_key_is_rejected() {
    let mut h2 = chrome_h2();
    h2.settings_order.push("initial_windowsize".into()); // typo of initial_window_size
    assert!(
        H2Config::from_profile(&h2).is_err(),
        "a typo'd settings_order key was silently dropped, shipping the wrong SETTINGS frame"
    );
}

// ── C-2: pseudo_order must be exactly four known tokens ──────────────────────
#[test]
fn unknown_pseudo_order_token_is_rejected() {
    let mut h2 = chrome_h2();
    h2.pseudo_order = vec![
        "method".into(),
        "xyz".into(), // unknown
        "authority".into(),
        "scheme".into(),
    ];
    assert!(
        H2Config::from_profile(&h2).is_err(),
        "an unknown pseudo_order token silently kept the Chrome default slot"
    );
}

#[test]
fn short_pseudo_order_is_rejected() {
    let mut h2 = chrome_h2();
    h2.pseudo_order = vec!["method".into(), "authority".into(), "scheme".into()]; // 3, not 4
    assert!(
        H2Config::from_profile(&h2).is_err(),
        "a 3-element pseudo_order silently padded the 4th slot with a Chrome default"
    );
}

#[test]
fn duplicate_pseudo_order_token_is_rejected() {
    let mut h2 = chrome_h2();
    // 4 entries, all individually valid — but `:method` twice means
    // `:authority` is missing; build_pseudo_list would emit a malformed
    // request (duplicate pseudo-header, one dropped entirely).
    h2.pseudo_order = vec![
        "method".into(),
        "method".into(),
        "scheme".into(),
        "path".into(),
    ];
    assert!(
        H2Config::from_profile(&h2).is_err(),
        "a duplicate pseudo_order token passed validation"
    );
}

// ── C-3: a missing connection window is a data bug, not RFC-default ──────────
#[test]
fn missing_connection_window_is_rejected() {
    let mut h2 = chrome_h2();
    h2.initial_connection_window_size = None;
    assert!(
        H2Config::from_profile(&h2).is_err(),
        "a missing initial_connection_window_size silently fell back to RFC 65535 \
         (so no WINDOW_UPDATE frame, Akamai fingerprint shows |0|)"
    );
}

// ── C-4: profile family must map to a real H3 config, never default ─────────
#[cfg(feature = "http3")]
#[test]
fn unknown_profile_family_has_no_h3_config() {
    use leyline::H3Config;
    // okhttp ships no HTTP/3 fingerprint; it must error, not borrow Chrome's.
    assert!(
        H3Config::for_family("okhttp").is_err(),
        "okhttp silently received an H3 config it has no fingerprint for"
    );
    // Empty family is the #[serde(default)] value — must not default to Chrome.
    assert!(
        H3Config::for_family("").is_err(),
        "empty meta.family silently defaulted to Chrome's QUIC transport params"
    );
}

#[cfg(feature = "http3")]
#[test]
fn gecko_family_maps_to_firefox_h3_not_chrome() {
    use leyline::H3Config;
    // Firefox profiles declare family="gecko" (not "firefox"); the old match
    // arm tested "firefox" and never fired, silently shipping Chrome's H3.
    let gecko = H3Config::for_family("gecko").expect("gecko maps to an H3 config");
    assert_eq!(
        gecko.initial_max_streams_bidi,
        H3Config::firefox().initial_max_streams_bidi,
        "gecko must resolve to Firefox H3 params, not Chrome's"
    );
    assert_ne!(
        gecko.initial_max_streams_bidi,
        H3Config::chrome().initial_max_streams_bidi,
        "gecko is still resolving to Chrome's H3 params"
    );
}

// ── C-7: a garbage cert-compression name is a profile typo → reject ─────────
#[test]
fn unknown_cert_compression_algorithm_is_rejected() {
    let mut profile = chrome_profile();
    // Not an RFC 8879 codepoint — a typo, not a real algorithm.
    profile.tls.cert_compression = vec!["frobnicate".into()];
    assert!(
        TlsContext::from_profile(&profile, TlsMinVersion::Tls13).is_err(),
        "a garbage cert-compression algorithm name was silently dropped from the ClientHello"
    );
}

// Real RFC 8879 codepoints (Firefox advertises zlib + brotli + zstd) must all
// build — each is now registered with a working decompressor, so the advertised
// `compress_certificate` extension is honest.
#[test]
fn real_cert_compression_codepoints_still_build() {
    let mut profile = chrome_profile();
    profile.tls.cert_compression = vec!["zlib".into(), "brotli".into(), "zstd".into()];
    assert!(
        TlsContext::from_profile(&profile, TlsMinVersion::Tls13).is_ok(),
        "a real RFC 8879 cert-compression list (as Firefox ships) failed to build"
    );
}

// ── A fixed extension order must be a complete, real permutation ──────
// BoringSSL rejects an unknown or repeated extension ID outright, and appends
// whatever the list omits in its own order. Either way the ClientHello stops
// matching the browser the profile claims to be — JA4_r drift the sorted JA4
// hides — so a broken order fails at load rather than at connect.
const FIREFOX_152: &str = include_str!("../profiles/firefox/152.toml");
const FIREFOX_152_PERMUTATION: &str =
    "[0, 23, 65281, 10, 11, 35, 16, 5, 34, 18, 51, 43, 13, 45, 28, 27, 65037]";

/// The real Firefox 152 profile with its captured extension order swapped out.
fn firefox_152_ordered(list: &str) -> String {
    let captured = format!("extension_permutation = {FIREFOX_152_PERMUTATION}");
    assert!(
        FIREFOX_152.contains(&captured),
        "firefox/152.toml no longer declares the captured permutation verbatim; \
         update FIREFOX_152_PERMUTATION"
    );
    FIREFOX_152.replace(&captured, &format!("extension_permutation = {list}"))
}

/// Load Firefox 152 under a broken order and return the rejection message.
fn permutation_load_error(list: &str) -> String {
    BrowserProfile::from_toml(&firefox_152_ordered(list))
        .expect_err("a broken extension_permutation loaded successfully")
        .to_string()
}

#[test]
fn permutation_entry_outside_the_advertised_set_is_rejected() {
    // 0x1234 is not an extension any profile advertises. BoringSSL would reject
    // the list wholesale and silently ship its default order.
    let err = permutation_load_error(
        "[0, 23, 65281, 10, 11, 35, 16, 5, 34, 18, 51, 43, 13, 45, 28, 27, 4660]",
    );
    assert!(
        err.contains("0x1234"),
        "rejection did not name the offending entry: {err}"
    );
}

#[test]
fn permutation_omitting_an_advertised_extension_is_rejected() {
    // compress_certificate (27) dropped while cert_compression stays populated:
    // BoringSSL appends it after the listed extensions, off its captured spot.
    let err = permutation_load_error(
        "[0, 23, 65281, 10, 11, 35, 16, 5, 34, 18, 51, 43, 13, 45, 28, 65037]",
    );
    assert!(
        err.contains("compress_certificate"),
        "rejection did not name the omitted extension: {err}"
    );
}

#[test]
fn permutation_with_a_repeated_extension_id_is_rejected() {
    let err = permutation_load_error(
        "[0, 0, 65281, 10, 11, 35, 16, 5, 34, 18, 51, 43, 13, 45, 28, 27, 65037]",
    );
    assert!(
        err.contains("repeats 0x0000"),
        "rejection did not name the repeated entry: {err}"
    );
}

#[test]
fn empty_permutation_is_rejected() {
    let err = permutation_load_error("[]");
    assert!(
        err.contains("empty"),
        "an empty extension_permutation was treated as 'no order declared': {err}"
    );
}

#[test]
fn positioning_pre_shared_key_is_rejected() {
    // 41 appears in a resumed-handshake capture, but TLS 1.3 fixes it last and
    // BoringSSL ignores any position given for it — listing it would promise a
    // wire order leyline cannot deliver.
    let err = permutation_load_error(
        "[0, 23, 65281, 10, 11, 35, 16, 5, 34, 18, 51, 43, 13, 45, 28, 27, 65037, 41]",
    );
    assert!(
        err.contains("pre_shared_key"),
        "rejection did not explain the pre_shared_key constraint: {err}"
    );
}

// Positive guard: the shipped orders survive the gate, and at least one profile
// actually exercises it (ProfileRegistry::builtin panics on a rejected profile).
#[test]
fn builtin_profiles_declaring_an_extension_order_still_load() {
    let reg = ProfileRegistry::builtin();
    let declaring = ALL_BROWSERS
        .into_iter()
        .filter(|browser| {
            reg.get_browser(*browser)
                .expect("built-in profile")
                .tls
                .extension_permutation
                .is_some()
        })
        .count();
    assert!(
        declaring > 0,
        "no built-in profile declares extension_permutation — the load-time gate is untested"
    );
}

// ── verified_at: every shipping profile must name what it was anchored to ────
// The convention used to be doc-only (CONTRIBUTING.md TODO). A profile whose
// fingerprint was never anchored against live browser output is exactly how the
// profile-drift soft-block incident shipped — so a missing `verified_against` is now a
// test failure. Date staleness belongs to the approved self-hosted capture;
// backfilling capture dates here would
// mean inventing dates we don't have, which is the false-anchor this guards.)
#[test]
fn every_builtin_profile_declares_verified_against() {
    let reg = ProfileRegistry::builtin();
    for browser in ALL_BROWSERS {
        let profile = reg.get_browser(browser).expect("built-in profile");
        assert!(
            !profile.meta.verified_against.trim().is_empty(),
            "{browser}: [meta] verified_against is empty — the profile fingerprint \
             was never anchored against a live capture (see CONTRIBUTING.md)"
        );
    }
}

#[test]
fn chrome150_identity_matches_capture_on_every_supported_platform() {
    const SEC_CH_UA: &str = r#""Not;A=Brand";v="8", "Chromium";v="150", "Google Chrome";v="150""#;
    let registry = ProfileRegistry::builtin();
    let profile = registry
        .get_browser(Browser::Chrome150)
        .expect("chrome150 built-in profile");

    for (platform, ua_marker) in [
        (Platform::Windows, "Windows NT 10.0"),
        (Platform::MacOS, "Macintosh; Intel Mac OS X 10_15_7"),
        (Platform::Linux, "X11; Linux x86_64"),
        (Platform::Android, "Linux; Android 14; Pixel 8"),
    ] {
        let identity = profile
            .identity_for(platform)
            .unwrap_or_else(|| panic!("Chrome 150 has no {platform:?} identity"));
        assert_eq!(identity.sec_ch_ua, SEC_CH_UA, "{platform:?} sec-ch-ua");
        assert!(
            identity.user_agent.contains(ua_marker),
            "{platform:?} UA has the wrong platform: {}",
            identity.user_agent
        );
        assert!(
            identity.user_agent.contains("Chrome/150.0.0.0"),
            "{platform:?} UA has the wrong Chrome major: {}",
            identity.user_agent
        );
    }
}

// ── Positive guard: every shipping profile must build an SSL context ─────────
#[test]
fn every_builtin_profile_builds_ssl_context() {
    let reg = ProfileRegistry::builtin();
    for browser in ALL_BROWSERS {
        let profile = reg.get_browser(browser).expect("built-in profile");
        for min in [TlsMinVersion::Tls12, TlsMinVersion::Tls13] {
            TlsContext::from_profile(profile, min).unwrap_or_else(|e| {
                panic!(
                    "{browser} failed TlsContext::from_profile (min {min:?}) after hardening: {e}"
                )
            });
        }
    }
}

// ── Positive guard: stricter validation must not reject any shipping profile ─
#[test]
fn every_builtin_h2config_resolves_on_all_platforms() {
    let reg = ProfileRegistry::builtin();
    for browser in ALL_BROWSERS {
        let profile = reg.get_browser(browser).expect("built-in profile");
        for platform in ALL_PLATFORMS {
            let resolved = profile
                .h2
                .resolve_for_platform(platform)
                .unwrap_or_else(|e| panic!("{browser} on {platform:?} failed to resolve: {e}"));
            let cfg = H2Config::from_profile(&resolved).unwrap_or_else(|e| {
                panic!("{browser} on {platform:?} failed H2 validation after hardening: {e}")
            });
            assert!(
                !cfg.settings.is_empty(),
                "{browser} on {platform:?} resolved to an empty SETTINGS frame"
            );
        }
    }
}

// ── -k escape hatch: building with verification disabled must not depend ─────
// on a loadable system trust store. On this CI host the
// store loads fine either way, so this only anchors the happy path; the real
// guarantee is that `danger_accept_invalid_certs` builds with
// `without_system_roots`, which never touches the store.
#[test]
fn danger_accept_invalid_certs_session_builds() {
    leyline::Session::builder()
        .browser(Browser::Chrome147)
        .danger_accept_invalid_certs(true)
        .build()
        .expect("-k session must build even when system trust is unavailable");
}
