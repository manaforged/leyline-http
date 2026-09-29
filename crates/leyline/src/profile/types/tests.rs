use super::*;
use crate::profile::{Browser, Platform, ProfileRegistry};

fn chrome_h2(version: Browser) -> H2Profile {
    ProfileRegistry::builtin()
        .get_browser(version)
        .expect("built-in profile")
        .h2
        .clone()
}

#[test]
fn unknown_omit_settings_name_is_rejected() {
    let mut h2 = chrome_h2(Browser::Chrome147);
    let over = H2PlatformOverride {
        omit_settings: vec!["not_a_setting".into()],
        ..Default::default()
    };
    h2.platforms
        .insert(Platform::Windows.identity_key().to_string(), over);
    assert!(
        h2.resolve_for_platform(Platform::Windows).is_err(),
        "a bogus omit_settings name was silently ignored instead of rejected"
    );
}

#[test]
fn builtin_platform_overrides_resolve_ok() {
    let h2 = chrome_h2(Browser::Chrome145);
    assert!(h2.resolve_for_platform(Platform::MacOS).is_ok());
}
