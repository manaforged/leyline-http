use leyline::h2::H2Config;
use leyline::profile::{H2Profile, ProfileRegistry};
use leyline::{Browser, BrowserProfile, Platform, TlsContext, TlsMinVersion};

const ALL_PLATFORMS: [Platform; 5] = [
    Platform::Windows,
    Platform::MacOS,
    Platform::Linux,
    Platform::Android,
    Platform::IOS,
];

fn chrome_h2() -> H2Profile {
    chrome_profile().h2
}

fn chrome_profile() -> BrowserProfile {
    ProfileRegistry::builtin()
        .get_browser(Browser::Chrome147)
        .expect("chrome147 built-in profile")
        .clone()
}

#[test]
fn unknown_settings_order_key_is_rejected() {
    let mut h2 = chrome_h2();
    h2.settings_order.push("initial_windowsize".into());
    assert!(
        H2Config::from_profile(&h2).is_err(),
        "a typo'd settings_order key was silently dropped, shipping the wrong SETTINGS frame"
    );
}

#[test]
fn unknown_pseudo_order_token_is_rejected() {
    let mut h2 = chrome_h2();
    h2.pseudo_order = vec![
        "method".into(),
        "xyz".into(),
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
    h2.pseudo_order = vec!["method".into(), "authority".into(), "scheme".into()];
    assert!(
        H2Config::from_profile(&h2).is_err(),
        "a 3-element pseudo_order silently padded the 4th slot with a Chrome default"
    );
}

#[test]
fn duplicate_pseudo_order_token_is_rejected() {
    let mut h2 = chrome_h2();
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

#[cfg(feature = "http3")]
#[test]
fn profile_without_h3_table_has_no_h3_config() {
    use leyline::H3Config;
    assert!(
        H3Config::from_profile(Browser::OkHttpAndroid10.profile()).is_err(),
        "okhttp silently received an H3 config it has no fingerprint for"
    );
    assert!(
        H3Config::from_profile(&BrowserProfile::bare()).is_err(),
        "a profile with no [h3] table silently defaulted to Chrome's QUIC transport params"
    );
}

#[cfg(feature = "http3")]
#[test]
fn firefox_profile_carries_firefox_h3_not_chrome() {
    use leyline::H3Config;
    let firefox = H3Config::from_profile(Browser::Firefox154.profile()).expect("firefox h3");
    let chrome = H3Config::from_profile(Browser::Chrome152.profile()).expect("chrome h3");
    assert_ne!(
        firefox.initial_max_data, chrome.initial_max_data,
        "firefox is still resolving to Chrome's H3 params"
    );
}

#[cfg(feature = "http3")]
#[test]
fn qpack() {
    let captured = [
        (Browser::Chrome145, 65536, 100),
        (Browser::Chrome146, 65536, 100),
        (Browser::Chrome147, 65536, 100),
        (Browser::Chrome148, 65536, 100),
        (Browser::Chrome149, 65536, 100),
        (Browser::Chrome150, 65536, 100),
        (Browser::Chrome151, 65536, 100),
        (Browser::Chrome152, 65536, 100),
        (Browser::Chrome153, 65536, 100),
        (Browser::Chrome154, 65536, 100),
        (Browser::Brave146, 65536, 100),
        (Browser::Brave154, 65536, 100),
        (Browser::Firefox148, 65536, 20),
        (Browser::Firefox149, 65536, 20),
        (Browser::Firefox150, 65536, 20),
        (Browser::Firefox151, 65536, 20),
        (Browser::Firefox152, 65536, 20),
        (Browser::Firefox153, 65536, 20),
        (Browser::Firefox154, 65536, 20),
        (Browser::Firefox155, 65536, 20),
        (Browser::Firefox156, 65536, 20),
        (Browser::Firefox157, 65536, 20),
        (Browser::Safari18, 16383, 100),
        (Browser::Safari26, 16383, 100),
        (Browser::Safari27, 16383, 100),
        (Browser::SafariIOS18, 16383, 100),
        (Browser::SafariIOS27, 16383, 100),
    ];
    for browser in Browser::all().iter().copied() {
        let Some(h3) = browser.profile().h3.as_ref() else {
            continue;
        };
        let setting = |id: u64| {
            h3.settings.as_ref().map_or(0, |list| {
                list.iter()
                    .find(|s| s.id == Some(id))
                    .and_then(|s| s.value)
                    .unwrap_or(0)
            })
        };
        let legacy = (
            h3.qpack_max_table_capacity.unwrap_or(0),
            h3.qpack_blocked_streams.unwrap_or(0),
        );
        let advertised = if h3.settings.is_some() {
            (setting(1), setting(7))
        } else {
            legacy
        };
        let expected = captured
            .iter()
            .find(|(b, _, _)| *b == browser)
            .map_or((0, 0), |(_, cap, blocked)| (*cap, *blocked));
        assert_eq!(advertised, expected, "{browser}");
    }
}

#[test]
fn unknown_cert_compression_algorithm_is_rejected() {
    let mut profile = chrome_profile();
    profile.tls.cert_compression = vec!["frobnicate".into()];
    assert!(
        TlsContext::from_profile(&profile, TlsMinVersion::Tls13).is_err(),
        "a garbage cert-compression algorithm name was silently dropped from the ClientHello"
    );
}

#[test]
#[cfg(all(
    feature = "compression-brotli",
    feature = "compression-zstd",
    any(feature = "compression-gzip", feature = "compression-deflate")
))]
fn real_cert_compression_codepoints_still_build() {
    let mut profile = chrome_profile();
    profile.tls.cert_compression = vec!["zlib".into(), "brotli".into(), "zstd".into()];
    assert!(
        TlsContext::from_profile(&profile, TlsMinVersion::Tls13).is_ok(),
        "a real RFC 8879 cert-compression list (as Firefox ships) failed to build"
    );
}

#[test]
#[cfg(not(feature = "compression-zstd"))]
fn cert_compression_without_its_feature_names_the_feature() {
    let mut profile = chrome_profile();
    profile.tls.cert_compression = vec!["zstd".into()];
    let err = match TlsContext::from_profile(&profile, TlsMinVersion::Tls13) {
        Err(err) => err,
        Ok(_) => panic!("zstd cert decompression built with `compression-zstd` off"),
    };
    assert!(
        err.to_string().contains("compression-zstd"),
        "the error must name the feature to enable: {err}"
    );
}

const FIREFOX_152: &str = include_str!("../profiles/firefox/152.toml");
const FIREFOX_152_PERMUTATION: &str =
    "[0, 23, 65281, 10, 11, 35, 16, 5, 34, 18, 51, 43, 13, 45, 28, 27, 65037]";

fn firefox_152_ordered(list: &str) -> String {
    let captured = format!("extension_permutation = {FIREFOX_152_PERMUTATION}");
    assert!(
        FIREFOX_152.contains(&captured),
        "firefox/152.toml no longer declares the captured permutation verbatim; \
         update FIREFOX_152_PERMUTATION"
    );
    FIREFOX_152.replace(&captured, &format!("extension_permutation = {list}"))
}

fn permutation_load_error(list: &str) -> String {
    BrowserProfile::from_toml(&firefox_152_ordered(list))
        .expect_err("a broken extension_permutation loaded successfully")
        .to_string()
}

#[test]
fn permutation_entry_outside_the_advertised_set_is_rejected() {
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
    let err = permutation_load_error(
        "[0, 23, 65281, 10, 11, 35, 16, 5, 34, 18, 51, 43, 13, 45, 28, 27, 65037, 41]",
    );
    assert!(
        err.contains("pre_shared_key"),
        "rejection did not explain the pre_shared_key constraint: {err}"
    );
}

#[test]
fn builtin_profiles_declaring_an_extension_order_still_load() {
    let reg = ProfileRegistry::builtin();
    let declaring = Browser::all()
        .iter()
        .copied()
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
