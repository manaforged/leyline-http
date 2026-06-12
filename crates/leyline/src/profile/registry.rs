//! Profile registry — loads and indexes browser profiles.

use std::collections::HashMap;

use crate::profile::types::BrowserProfile;
use crate::profile::Browser;

/// Registry of all loaded browser profiles, indexed by (browser, version).
pub struct ProfileRegistry {
    profiles: HashMap<(String, u32), BrowserProfile>,
}

impl ProfileRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            profiles: HashMap::new(),
        }
    }

    /// Load all built-in profiles (compiled in via include_str!).
    pub fn builtin() -> Self {
        let mut reg = Self::new();
        // Chrome
        reg.load_toml(include_str!("../../profiles/chrome/145.toml"));
        reg.load_toml(include_str!("../../profiles/chrome/146.toml"));
        reg.load_toml(include_str!("../../profiles/chrome/147.toml"));
        reg.load_toml(include_str!("../../profiles/chrome/148.toml"));
        // Aloha (Chromium-based)
        reg.load_toml(include_str!("../../profiles/aloha/138.toml"));
        // Brave (Chromium-based, first-class because of header-order +
        // sec-gpc + accept SXG-strip deltas)
        reg.load_toml(include_str!("../../profiles/brave/146.toml"));
        // Firefox
        reg.load_toml(include_str!("../../profiles/firefox/148.toml"));
        reg.load_toml(include_str!("../../profiles/firefox/150.toml"));
        reg.load_toml(include_str!("../../profiles/firefox/151.toml"));
        // Safari
        reg.load_toml(include_str!("../../profiles/safari/18.toml"));
        reg.load_toml(include_str!("../../profiles/safari/ios15.toml"));
        reg.load_toml(include_str!("../../profiles/safari/ios17.toml"));
        reg.load_toml(include_str!("../../profiles/safari/ios18.toml"));
        // OkHttp
        reg.load_toml(include_str!("../../profiles/okhttp/android10.toml"));
        reg.load_toml(include_str!("../../profiles/okhttp/android7.toml"));
        reg
    }

    /// Parse and insert a TOML profile string.
    ///
    /// Panics on parse failure: every caller feeds `include_str!`
    /// compile-time constants, so a bad profile is a programmer error
    /// (merge conflict, hand-edit). The old `tracing::error!` + skip
    /// deferred the failure to a misleading "built-in profile missing"
    /// panic at the `profile()` call site, with the parse error buried
    /// in trace output.
    fn load_toml(&mut self, toml_str: &str) {
        let profile = BrowserProfile::from_toml(toml_str)
            .unwrap_or_else(|e| panic!("built-in profile failed to parse: {e}"));
        let key = (profile.meta.browser.clone(), profile.meta.version);
        self.profiles.insert(key, profile);
    }

    /// Look up a profile by browser name and version.
    pub fn get(&self, browser: &str, version: u32) -> Option<&BrowserProfile> {
        self.profiles.get(&(browser.to_string(), version))
    }

    /// Look up a profile by Browser enum.
    pub fn get_browser(&self, browser: Browser) -> Option<&BrowserProfile> {
        let (name, version) = browser.profile_key();
        self.get(name, version)
    }

    /// Number of loaded profiles.
    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }
}

impl Default for ProfileRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Built-ins are include_str! compile-time constants — a parse
    // failure is a programmer error (bad merge, hand-edit) and must
    // fail at load with the parse error, not surface 30 calls later
    // as a misleading "built-in profile missing" panic.
    #[test]
    #[should_panic(expected = "built-in profile failed to parse")]
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
        assert_eq!(profile.tls.ciphers.len(), 15);
        assert_eq!(
            profile.h2.pseudo_order,
            vec!["method", "path", "authority", "scheme"]
        );
        assert_eq!(
            profile.expected_ja4(),
            Some("t13d1517h2_8daaf6152771_68c5a8c2958d")
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
}
