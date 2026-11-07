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
fn latest_is_the_newest_browser_capture() {
    let meta = |browser: &Browser| -> (bool, bool) {
        let table: toml::Table = toml::from_str(browser.entry().source).unwrap();
        let meta = &table["meta"];
        (
            meta.get("capture").and_then(toml::Value::as_str) == Some("browser"),
            meta.get("deprecated").is_none(),
        )
    };
    for family in ALL_FAMILIES {
        let latest = Browser::latest(family);
        let version = latest.version();
        let (latest_captured, _) = meta(&latest);
        let better = Browser::all().iter().find(|b| {
            let (captured, live) = meta(b);
            b.family() == family
                && b.brand().is_none()
                && live
                && if latest_captured {
                    captured && b.version() > version
                } else {
                    captured || b.version() > version
                }
        });
        assert!(
            better.is_none(),
            "{family} latest is {latest}, but {} is the newer browser capture",
            better.map(ToString::to_string).unwrap_or_default()
        );
    }
}

#[test]
fn latest_covers_every_bundled_family() {
    for browser in Browser::all() {
        let family = browser.family();
        let named = ALL_FAMILIES
            .iter()
            .any(|f| Browser::latest(*f).family() == family);
        assert!(named, "no Family variant reaches the {family:?} profiles");
    }
}

#[test]
fn latest_matches_the_builder_defaults() {
    assert_eq!(Browser::chrome(), Browser::default());
    assert_eq!(Browser::edge().base(), Browser::chrome());
    assert_eq!(Browser::edge().version(), Browser::chrome().version());
}

#[test]
fn host_resolves_to_the_running_platform() {
    for browser in Browser::all() {
        assert_eq!(
            browser.for_platform(Platform::Host),
            browser.for_platform(Platform::detect_host()),
            "{browser}"
        );
    }
}

#[test]
fn a_bare_profile_edit_does_not_change_a_browser_profile_id() {
    let browser = Browser::Chrome146;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("profiles");
    let shared: Vec<Vec<u8>> = ["brands.toml", "headers.toml", "platforms.toml"]
        .into_iter()
        .map(|name| std::fs::read(root.join(name)).unwrap())
        .collect();
    let source = browser.profile_source();
    let mut parts: Vec<&[u8]> = shared.iter().map(Vec::as_slice).collect();
    parts.push(source.as_bytes());
    assert_eq!(browser.profile_digest(), super::digest(&parts));

    let bare = std::fs::read(root.join("bare.toml")).unwrap();
    let mut with_bare = vec![bare.as_slice()];
    with_bare.extend(parts);
    assert_ne!(browser.profile_digest(), super::digest(&with_bare));
}

#[test]
fn the_bare_profile_advertises_64_kib_http2_frames() {
    let profile = crate::profile::BrowserProfile::bare_shared();
    assert_eq!(
        profile
            .h2
            .settings
            .get(crate::profile::SettingId::MaxFrameSize),
        Some(65_536)
    );
}

#[test]
fn every_bundled_identity_sends_an_accept_language() {
    let platforms = [
        Platform::Windows,
        Platform::MacOS,
        Platform::Linux,
        Platform::Android,
        Platform::IOS,
    ];
    for browser in Browser::all() {
        for platform in platforms {
            if let Some(identity) = browser.identity_headers(platform) {
                assert!(
                    identity.accept_language.is_some(),
                    "{browser} on {platform:?}"
                );
            }
        }
    }
}
