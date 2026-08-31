use super::*;
use crate::profile::ProfileRegistry;

#[test]
fn chrome147_h2_fingerprint() {
    let reg = ProfileRegistry::builtin();
    let profile = reg.get("chrome", 147).unwrap();
    let h2 = H2Config::from_profile(&profile.h2).unwrap();
    let fp = h2.akamai_fingerprint();
    assert_eq!(fp, "1:65536;2:0;4:6291456;6:262144|15663105|0|m,a,s,p");
}

#[test]
fn chrome148_h2_fingerprint() {
    let reg = ProfileRegistry::builtin();
    let profile = reg.get("chrome", 148).unwrap();
    let h2 = H2Config::from_profile(&profile.h2).unwrap();
    let fp = h2.akamai_fingerprint();
    assert_eq!(fp, "1:65536;2:0;4:6291456;6:262144|15663105|0|m,a,s,p");
}

#[test]
fn chrome150_priority() {
    let reg = ProfileRegistry::builtin();
    let profile = reg.get("chrome", 150).unwrap();
    assert!(profile.h2.default_priority.is_some());
    let h2 = H2Config::from_profile(&profile.h2).unwrap();
    assert!(h2.default_priority.is_some());
    let fp = h2.akamai_fingerprint();
    assert_eq!(fp, "1:65536;2:0;4:6291456;6:262144|15663105|0|m,a,s,p");
}

#[test]
fn firefox150_h2_fingerprint() {
    let reg = ProfileRegistry::builtin();
    let profile = reg.get("firefox", 150).unwrap();
    let h2 = H2Config::from_profile(&profile.h2).unwrap();
    let fp = h2.akamai_fingerprint();
    assert_eq!(fp, "1:65536;2:0;4:131072;5:16384|12517377|0|m,p,a,s");
}

#[test]
fn okhttp_h2_fingerprint() {
    let reg = ProfileRegistry::builtin();
    let profile = reg.get("okhttp", 10).unwrap();
    let h2 = H2Config::from_profile(&profile.h2).unwrap();
    let fp = h2.akamai_fingerprint();
    assert_eq!(fp, "4:16777216|16711681|0|m,p,a,s");
}

#[test]
fn safari18_h2_fingerprint() {
    let reg = ProfileRegistry::builtin();
    let profile = reg.get("safari", 18).unwrap();
    let h2 = H2Config::from_profile(&profile.h2).unwrap();
    let fp = h2.akamai_fingerprint();
    assert_eq!(fp, "2:0;3:100;4:2097152;8:1;9:1|10420225|0|m,s,a,p");
}

#[test]
fn safari_ios18_h2_fingerprint() {
    let reg = ProfileRegistry::builtin();
    let profile = reg.get("safari-ios", 18).unwrap();
    let h2 = H2Config::from_profile(&profile.h2).unwrap();
    let fp = h2.akamai_fingerprint();
    assert_eq!(fp, "2:0;3:100;4:2097152;9:1|10420225|0|m,s,a,p");
}

#[test]
fn all_profiles_produce_expected_fingerprint() {
    let reg = ProfileRegistry::builtin();
    for browser in [
        ("chrome", 145),
        ("chrome", 146),
        ("chrome", 147),
        ("chrome", 148),
        ("chrome", 149),
        ("chrome", 150),
        ("firefox", 148),
        ("firefox", 150),
        ("firefox", 151),
        ("firefox", 152),
        ("safari", 18),
        ("safari-ios", 17),
        ("safari-ios", 18),
        ("cfnetwork-ios", 18),
        ("cfnetwork-macos", 26),
        ("okhttp", 10),
    ] {
        let profile = reg
            .get(browser.0, browser.1)
            .unwrap_or_else(|| panic!("missing profile: {} {}", browser.0, browser.1));
        if let Some(expected) = profile.expected_h2_fingerprint() {
            let h2 = H2Config::from_profile(&profile.h2).unwrap();
            let actual = h2.akamai_fingerprint();
            assert_eq!(
                actual, expected,
                "H2 fingerprint mismatch for {} {}",
                browser.0, browser.1
            );
        }
    }
}
