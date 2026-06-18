//! Negative-path validation for browser profiles.
//!
//! A fingerprinting library must fail loudly when profile data is malformed:
//! a silently-substituted default ships a wrong wire fingerprint, which CDN/WAF
//! edges soft-block (a profile-drift soft-block incident). Each test mutates
//! one field of a real built-in profile and asserts `H2Config::from_profile`
//! rejects it instead of degrading silently. The positive guard proves the
//! stricter validation does not reject any shipping profile.

use leyline::h2::H2Config;
use leyline::profile::{BrowserProfile, H2Profile, ProfileRegistry, ALL_BROWSERS};
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

// ── verified_at: every shipping profile must name what it was anchored to ────
// The convention used to be doc-only (CONTRIBUTING.md TODO). A profile whose
// fingerprint was never anchored against live browser output is exactly how the
// profile-drift soft-block incident shipped — so a missing `verified_against` is now a
// test failure. (Date-staleness enforcement is the weekly fingerprint-cron's
// job — it re-anchors against live truth; backfilling capture dates here would
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
