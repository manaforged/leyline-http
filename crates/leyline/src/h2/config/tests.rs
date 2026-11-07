use super::*;
use crate::profile::{ProfileRegistry, akamai_fingerprint};

fn akamai(h2: &H2Config) -> String {
    akamai_fingerprint(
        &h2.settings,
        h2.initial_connection_window_size,
        &h2.pseudo_order,
    )
}

#[test]
fn chrome150_priority() {
    let reg = ProfileRegistry::builtin();
    let profile = reg.get("chrome", 150).unwrap();
    assert!(profile.h2.default_priority.is_some());
    let h2 = H2Config::from_profile(&profile.h2).unwrap();
    assert!(h2.default_priority.is_some());
}

#[test]
fn all_profiles_produce_expected_fingerprint() {
    let reg = ProfileRegistry::builtin();
    let mut checked = 0;
    for browser in crate::profile::Browser::all().iter().copied() {
        let profile = reg
            .get_browser(browser)
            .unwrap_or_else(|| panic!("missing profile: {browser}"));
        let Some(expected) = profile
            .h2
            .fingerprint
            .as_ref()
            .and_then(|f| f.akamai.as_deref())
        else {
            continue;
        };
        let h2 = H2Config::from_profile(&profile.h2).unwrap();
        assert_eq!(
            akamai(&h2),
            expected,
            "H2 fingerprint mismatch for {browser}"
        );
        checked += 1;
    }
    assert!(checked > 0, "no profile declares an Akamai fingerprint");
}
