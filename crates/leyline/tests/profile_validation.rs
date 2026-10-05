use leyline::h2::H2Config;
use leyline::profile::{H2Profile, ProfileRegistry};
use leyline::{Browser, BrowserProfile, TlsContext, TlsMinVersion};

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
    let captured: std::collections::BTreeMap<String, (u64, u64)> =
        toml::from_str(include_str!("data/h3_qpack.toml"))
            .expect("tests/data/h3_qpack.toml parses");
    for key in captured.keys() {
        assert!(
            Browser::all().iter().any(|browser| browser.id() == key),
            "h3_qpack.toml names {key}, which is not a bundled profile"
        );
    }
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
        let expected = captured.get(browser.id()).copied().unwrap_or((0, 0));
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
