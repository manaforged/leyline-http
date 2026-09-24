use super::*;

const ALL_FAMILIES: [Family; 7] = [
    Family::Chrome,
    Family::Brave,
    Family::Firefox,
    Family::Safari,
    Family::SafariIos,
    Family::CfNetwork,
    Family::OkHttp,
];

#[test]
fn latest_is_the_highest_bundled_version() {
    for family in ALL_FAMILIES {
        let latest = Browser::latest(family);
        let (key, version) = latest.profile_key();
        let higher = Browser::all()
            .iter()
            .filter(|b| b.profile_key().0 == key)
            .find(|b| b.profile_key().1 > version);
        assert!(
            higher.is_none(),
            "{family} latest is {latest}, but {} is bundled and newer",
            higher.map(ToString::to_string).unwrap_or_default()
        );
    }
}

#[test]
fn latest_covers_every_bundled_family() {
    for browser in Browser::all() {
        let key = browser.family();
        let named = ALL_FAMILIES.iter().any(|f| {
            let latest = Browser::latest(*f);
            latest.family() == key
                || latest.for_platform(crate::profile::Platform::IOS).family() == key
        });
        assert!(named, "no Family variant reaches the {key} profiles");
    }
}

#[test]
fn latest_matches_the_builder_defaults() {
    assert_eq!(Browser::latest(Family::Chrome), Browser::default_browser());
    assert_eq!(Browser::latest(Family::Firefox), Browser::default_firefox());
}
