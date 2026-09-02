use super::*;

/// Smallest profile TOML the loader accepts, for the directory-load tests.
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

/// A fresh empty directory under the system temp dir.
fn scratch(name: &str) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("leyline-{name}-{stamp}"));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[test]
fn load_reads_a_family_directory() {
    let dir = scratch("load");
    let family = dir.join("test");
    std::fs::create_dir_all(&family).expect("create family dir");
    std::fs::write(family.join("1.toml"), MINIMAL).expect("write profile");

    let reg = ProfileRegistry::load(&dir).expect("load scratch registry");
    assert_eq!(reg.len(), 1);
    let profile = reg.get("test", 1).expect("test 1 not found");
    assert_eq!(profile.meta.name, "Test 1");
    assert_eq!(profile.tls.ciphers, vec!["TLS_AES_128_GCM_SHA256"]);

    std::fs::remove_dir_all(&dir).expect("clean scratch dir");
}

#[test]
fn load_rejects_an_invalid_permutation() {
    let dir = scratch("invalid");
    let family = dir.join("test");
    std::fs::create_dir_all(&family).expect("create family dir");
    let bad = MINIMAL.replace(
        "sigalgs = [\"ecdsa_secp256r1_sha256\"]",
        "sigalgs = [\"ecdsa_secp256r1_sha256\"]\nextension_permutation = [0]",
    );
    std::fs::write(family.join("1.toml"), bad).expect("write profile");

    match ProfileRegistry::load(&dir) {
        Err(ProfileError::Parse { .. }) => {}
        Err(other) => panic!("expected a Parse error, got {other}"),
        Ok(_) => panic!("permutation must be validated"),
    }

    std::fs::remove_dir_all(&dir).expect("clean scratch dir");
}

#[test]
fn load_rejects_an_empty_directory() {
    let dir = scratch("empty");
    match ProfileRegistry::load(&dir) {
        Err(ProfileError::Empty { .. }) => {}
        Err(other) => panic!("expected an Empty error, got {other}"),
        Ok(_) => panic!("empty directory must not load"),
    }
    std::fs::remove_dir_all(&dir).expect("clean scratch dir");
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
