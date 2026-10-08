use leyline::h2::H2Config;
use leyline::profile::ProfileRegistry;
use leyline::{Browser, BrowserProfile, Platform, TlsContext, TlsMinVersion};

const ALL_PLATFORMS: [Platform; 5] = [
    Platform::Windows,
    Platform::MacOS,
    Platform::Linux,
    Platform::Android,
    Platform::IOS,
];

#[test]
fn every_builtin_profile_declares_verified_against() {
    let reg = ProfileRegistry::builtin();
    for browser in Browser::all().iter().copied() {
        let profile = reg.get_browser(browser).expect("built-in profile");
        assert!(
            !profile.meta.verified_against.trim().is_empty(),
            "{browser}: [meta] verified_against is empty — the profile fingerprint \
             was never anchored against a live capture"
        );
    }
}

fn minimal_profile_toml(extra: &str) -> String {
    format!(
        r#"
[meta]
name = "Test Profile"
browser = "test"
version = 1
{extra}

[tls]
ciphers = ["TLS_AES_128_GCM_SHA256"]
curves = ["X25519"]
sigalgs = ["ecdsa_secp256r1_sha256"]

[h2]
pseudo_order = ["method", "scheme", "authority", "path"]
settings_order = ["header_table_size"]
"#
    )
}

#[test]
fn captured_against_parses_and_clears_the_warning() {
    let toml = minimal_profile_toml(r#"captured_against = "chrome-150.0.7871.128""#);
    let profile = BrowserProfile::from_toml(&toml).expect("minimal profile parses");
    assert_eq!(
        profile.meta.captured_against.as_deref(),
        Some("chrome-150.0.7871.128"),
        "captured_against did not round-trip from the TOML"
    );
    assert!(
        profile.load_warnings().is_empty(),
        "a profile that records captured_against still warned: {:?}",
        profile.load_warnings()
    );
}

#[test]
fn missing_captured_against_warns_but_still_loads() {
    let profile = BrowserProfile::from_toml(&minimal_profile_toml(""))
        .expect("a profile without captured_against must still load");
    assert!(profile.meta.captured_against.is_none());
    assert!(
        profile
            .load_warnings()
            .iter()
            .any(|w| w.contains("captured_against")),
        "a missing captured_against produced no warning: {:?}",
        profile.load_warnings()
    );

    let blank = BrowserProfile::from_toml(&minimal_profile_toml(r#"captured_against = "  ""#))
        .expect("a blank captured_against must still load");
    assert!(
        blank
            .load_warnings()
            .iter()
            .any(|w| w.contains("captured_against")),
        "a blank captured_against produced no warning: {:?}",
        blank.load_warnings()
    );
}

#[test]
fn backfilled_builtin_profiles_carry_captured_against() {
    let reg = ProfileRegistry::builtin();
    for (browser, expected) in [
        (Browser::Chrome148, "chrome-148.0.7778.216"),
        (Browser::Chrome150, "chrome-150.0.7871.187"),
        (Browser::Chrome151, "chrome-151.0.7922.174"),
        (Browser::Chrome152, "chrome-152.0.7977.83"),
        (Browser::Chrome153, "chrome-153.0.8010.53"),
        (Browser::Firefox148, "firefox-148.0.2"),
        (Browser::Firefox150, "firefox-150.0"),
        (Browser::Firefox151, "firefox-151.0.4"),
        (Browser::Firefox152, "firefox-152.0.6"),
        (Browser::Firefox153, "firefox-153.0.4"),
        (Browser::Firefox154, "firefox-154.0.1"),
        (Browser::Safari26, "safari-26.6.2-21624.5.1.11.3"),
        (Browser::Brave146, "brave-146.1.88.138"),
    ] {
        let profile = reg.get_browser(browser).expect("built-in profile");
        assert_eq!(
            profile.meta.captured_against.as_deref(),
            Some(expected),
            "{browser}: backfilled captured_against changed or was dropped"
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
        (Platform::Android, "Linux; Android 10; K"),
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

#[test]
#[cfg(all(
    feature = "compression-brotli",
    feature = "compression-zstd",
    any(feature = "compression-gzip", feature = "compression-deflate")
))]
fn every_builtin_profile_builds_ssl_context() {
    let reg = ProfileRegistry::builtin();
    for browser in Browser::all().iter().copied() {
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

#[test]
fn every_builtin_h2config_resolves_on_all_platforms() {
    let reg = ProfileRegistry::builtin();
    for browser in Browser::all().iter().copied() {
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

#[test]
#[cfg(all(
    feature = "compression-brotli",
    feature = "compression-zstd",
    any(feature = "compression-gzip", feature = "compression-deflate")
))]
fn danger_accept_invalid_certs_session_builds() {
    leyline::Session::builder()
        .browser(Browser::Chrome147)
        .tls_trust(leyline::TlsTrustConfig::new().danger_accept_invalid_certs(true))
        .build()
        .expect("-k session must build even when system trust is unavailable");
}

#[test]
fn cfnetwork_profiles_declare_captured_against() {
    let reg = ProfileRegistry::builtin();
    for browser in [Browser::CfnetworkIOS18, Browser::CfnetworkMacOS26] {
        let profile = reg.get_browser(browser).expect("built-in profile");
        assert_eq!(profile.meta.family, "cfnetwork");
        assert!(
            !profile
                .meta
                .captured_against
                .as_deref()
                .unwrap_or("")
                .trim()
                .is_empty(),
            "{browser}: cfnetwork profile must record captured_against \
             (exact CFNetwork build) — shipped profiles derive only from captures"
        );
    }
}

#[test]
fn cfnetwork_sigalgs_carry_the_wire_duplicate() {
    let reg = ProfileRegistry::builtin();
    for browser in [Browser::CfnetworkIOS18, Browser::CfnetworkMacOS26] {
        let profile = reg.get_browser(browser).expect("built-in profile");
        let dup = profile
            .tls
            .sigalgs
            .iter()
            .filter(|s| s.as_str() == "rsa_pss_rsae_sha384")
            .count();
        assert_eq!(
            dup, 2,
            "{browser}: CFNetwork sigalgs must duplicate rsa_pss_rsae_sha384 \
             (0x0805) exactly once (captured on the wire)"
        );
    }
}

#[test]
fn cfnetwork_ios18_lowers_version_floor_but_macos26_does_not() {
    let reg = ProfileRegistry::builtin();
    let ios = reg.get_browser(Browser::CfnetworkIOS18).expect("profile");
    assert_eq!(
        ios.tls.min_tls_version.as_deref(),
        Some("1.0"),
        "iOS 18.6 CFNetwork advertises TLS 1.0/1.1 in supported_versions"
    );
    let macos = reg.get_browser(Browser::CfnetworkMacOS26).expect("profile");
    assert_eq!(
        macos.tls.min_tls_version.as_deref(),
        None,
        "macOS 26 CFNetwork advertises only TLS 1.3/1.2"
    );
}

#[test]
fn cfnetwork_profiles_disable_session_tickets() {
    let reg = ProfileRegistry::builtin();
    for browser in [Browser::CfnetworkIOS18, Browser::CfnetworkMacOS26] {
        let profile = reg.get_browser(browser).expect("built-in profile");
        assert!(
            !profile.tls.session_tickets,
            "{browser}: CFNetwork sends no session_ticket extension on fresh \
             connections (captured)"
        );
    }
}

fn newest_chrome_browser() -> Browser {
    Browser::all()
        .iter()
        .copied()
        .filter_map(|b| b.chromium_major().map(|m| (b, m)))
        .max_by_key(|(_, m)| *m)
        .map(|(b, _)| b)
        .expect("at least one Chrome-family profile in Browser::all()")
}

#[test]
fn newest_chrome_profile_metadata_is_self_consistent() {
    let reg = ProfileRegistry::builtin();
    let browser = newest_chrome_browser();
    let major = browser.chromium_major().expect("chrome major");
    let newest = reg
        .get_browser(browser)
        .expect("newest chrome built-in profile");
    assert_eq!(
        newest.meta.version, major,
        "meta.version must match the profile major"
    );
    let anchor = newest
        .meta
        .captured_against
        .as_ref()
        .expect("newest Chrome must pin captured_against (corpus rot gate)");
    let anchor_major = anchor
        .split('-')
        .find(|part| part.as_bytes().first().is_some_and(u8::is_ascii_digit))
        .and_then(|part| part.split('.').next())
        .and_then(|m| m.parse::<u32>().ok());
    assert_eq!(
        anchor_major,
        Some(major),
        "captured_against must pin the same major as meta.version: {anchor}"
    );
}

#[test]
fn newest_chrome_profile_is_not_neglected() {
    const FRESHNESS_FLOOR: u32 = 145;
    let major = newest_chrome_browser()
        .chromium_major()
        .expect("chrome major");
    assert!(
        major >= FRESHNESS_FLOOR,
        "newest Chrome profile is {major} (floor {FRESHNESS_FLOOR}) — the corpus is \
         neglected; ship a current capture"
    );
}

#[test]
#[ignore = "network: fetches Chrome for Testing; run on schedule"]
fn newest_chrome_profile_within_two_majors_of_current_stable() {
    const URL: &str = "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json";
    let out = std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "20", URL])
        .output()
        .expect("curl must be available for the live freshness gate");
    assert!(out.status.success(), "Chrome for Testing fetch failed");
    let json: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("parse Chrome for Testing json");
    let stable = json["channels"]["Stable"]["version"]
        .as_str()
        .expect("Stable version string");
    let current: u32 = stable
        .split('.')
        .next()
        .expect("major")
        .parse()
        .expect("numeric major");
    let newest_major = newest_chrome_browser()
        .chromium_major()
        .expect("chrome major");
    assert!(
        current <= newest_major + 1,
        "newest built-in Chrome profile is {newest_major}, current stable is \
         {current} — more than one major behind; ship a fresh capture"
    );
}
