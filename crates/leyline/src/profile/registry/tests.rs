use super::*;

// Built-ins are include_str! compile-time constants — a parse
// failure is a programmer error (bad merge, hand-edit) and must
// fail at load with the parse error, not surface 30 calls later
// as a misleading "built-in profile missing" panic.
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
        reg.len(),
        crate::profile::PROFILE_COUNT,
        "registry count != PROFILE_COUNT constant"
    );
}

#[test]
fn every_browser_variant_resolves() {
    let reg = ProfileRegistry::builtin();
    for browser in crate::profile::ALL_BROWSERS {
        assert!(
            reg.get_browser(browser).is_some(),
            "no profile for {browser}"
        );
    }
}

#[test]
fn every_profile_has_fingerprint() {
    let reg = ProfileRegistry::builtin();
    for browser in crate::profile::ALL_BROWSERS {
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
fn chrome147_profile_parses() {
    let reg = ProfileRegistry::builtin();
    let profile = reg.get("chrome", 147).expect("chrome 147 not found");
    assert_eq!(profile.meta.name, "Chrome 147");
    assert_eq!(profile.tls.ciphers.len(), 15);
    assert_eq!(profile.tls.curves.len(), 4);
    assert!(profile.tls.permute_extensions);
    assert!(profile.tls.ech_grease);
    assert_eq!(
        profile.h2.pseudo_order,
        vec!["method", "authority", "scheme", "path"]
    );
    assert!(profile.identity.contains_key("windows"));
}

#[test]
fn firefox148_has_extension_permutation() {
    let reg = ProfileRegistry::builtin();
    let profile = reg.get("firefox", 148).expect("firefox 148 not found");
    assert!(profile.tls.extension_permutation.is_some());
    assert_eq!(profile.tls.ciphers.len(), 17);
    assert_eq!(
        profile.h2.pseudo_order,
        vec!["method", "path", "authority", "scheme"]
    );
}

#[test]
fn firefox151_profile_parses() {
    let reg = ProfileRegistry::builtin();
    let profile = reg.get("firefox", 151).expect("firefox 151 not found");
    assert_eq!(profile.meta.name, "Firefox 151");
    assert_eq!(profile.tls.ciphers.len(), 16);
    assert_eq!(
        profile.h2.pseudo_order,
        vec!["method", "path", "authority", "scheme"]
    );
    assert_eq!(
        profile.expected_ja4(),
        Some("t13d1617h2_86a278354501_3cbfd9057e0d")
    );
}

#[test]
fn firefox152_profile_parses() {
    let reg = ProfileRegistry::builtin();
    let profile = reg.get("firefox", 152).expect("firefox 152 not found");
    assert_eq!(profile.meta.name, "Firefox 152");
    assert_eq!(profile.tls.ciphers.len(), 16);
    // Real FF152.0 == real FF151.0 at the TLS layer.
    assert_eq!(
        profile.expected_ja4(),
        Some("t13d1617h2_86a278354501_3cbfd9057e0d")
    );
}

#[test]
fn firefox150_profile_parses() {
    let reg = ProfileRegistry::builtin();
    let profile = reg.get("firefox", 150).expect("firefox 150 not found");
    assert_eq!(profile.meta.name, "Firefox 150");
    assert_eq!(profile.tls.ciphers.len(), 17);
    assert_eq!(
        profile.h2.pseudo_order,
        vec!["method", "path", "authority", "scheme"]
    );
    assert_eq!(
        profile.expected_ja4(),
        Some("t13d1717h2_5b57614c22b0_3cbfd9057e0d")
    );
    assert_eq!(
        profile.expected_resumed_ja4(),
        Some("t13d1717h2_5b57614c22b0_e6dcd7ae0a9e")
    );
}
