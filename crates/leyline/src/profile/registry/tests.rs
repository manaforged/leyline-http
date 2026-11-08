use super::*;

const MINIMAL: &str = r#"
[meta]
name = "Test 1"
browser = "test"
version = 1
family = "chromium"
captured_against = "test-1.0"

[tls]
ciphers = ["TLS_AES_128_GCM_SHA256"]
curves = ["X25519"]
sigalgs = ["ecdsa_secp256r1_sha256"]

[h2]
pseudo_order = ["method", "authority", "scheme", "path"]
settings_order = ["header_table_size"]
"#;

#[test]
fn from_toml_rejects_an_invalid_permutation() {
    let bad = MINIMAL.replace(
        "sigalgs = [\"ecdsa_secp256r1_sha256\"]",
        "sigalgs = [\"ecdsa_secp256r1_sha256\"]\nextension_permutation = [0]",
    );
    let err = BrowserProfile::from_toml(&bad).expect_err("permutation must be validated");
    assert!(err.to_string().contains("invalid profile"), "{err}");
}

#[test]
#[should_panic(expected = "built-in profile is statically valid")]
fn malformed_builtin_toml_panics_at_load() {
    let mut reg = ProfileRegistry::new();
    reg.load_toml("this is not a browser profile");
}

#[test]
fn builtin_loads_all_profiles() {
    let reg = ProfileRegistry::builtin();
    assert_eq!(
        reg.profiles.len(),
        crate::profile::Browser::all()
            .iter()
            .filter(|browser| browser.brand().is_none())
            .count(),
        "registry count != Browser::all() length"
    );
}

#[test]
fn every_browser_variant_resolves() {
    let reg = ProfileRegistry::builtin();
    for browser in crate::profile::Browser::all().iter().copied() {
        assert!(
            reg.get_browser(browser).is_some(),
            "no profile for {browser}"
        );
    }
}

#[test]
fn every_profile_has_fingerprint() {
    let reg = ProfileRegistry::builtin();
    for browser in crate::profile::Browser::all().iter().copied() {
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
