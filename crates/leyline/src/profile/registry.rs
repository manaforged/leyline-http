//! Profile registry — loads and indexes browser profiles.

use std::collections::HashMap;
use std::sync::LazyLock;

use crate::profile::Browser;
use crate::profile::types::BrowserProfile;

/// Registry of all loaded browser profiles, indexed by (browser, version).
pub struct ProfileRegistry {
    profiles: HashMap<(String, u32), BrowserProfile>,
}

static BUILTIN: LazyLock<ProfileRegistry> = LazyLock::new(ProfileRegistry::builtin);

impl ProfileRegistry {
    /// The compiled-in profile set.
    #[must_use]
    pub fn global() -> &'static Self {
        &BUILTIN
    }

    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            profiles: HashMap::new(),
        }
    }

    /// Load all built-in profiles (compiled in via include_str!).
    pub fn builtin() -> Self {
        let mut reg = Self::new();
        reg.load_toml(include_str!("../../profiles/chrome/145.toml"));
        reg.load_toml(include_str!("../../profiles/chrome/146.toml"));
        reg.load_toml(include_str!("../../profiles/chrome/147.toml"));
        reg.load_toml(include_str!("../../profiles/chrome/148.toml"));
        reg.load_toml(include_str!("../../profiles/chrome/149.toml"));
        reg.load_toml(include_str!("../../profiles/chrome/150.toml"));
        reg.load_toml(include_str!("../../profiles/chrome/151.toml"));
        reg.load_toml(include_str!("../../profiles/chrome/152.toml"));
        reg.load_toml(include_str!("../../profiles/brave/146.toml"));
        reg.load_toml(include_str!("../../profiles/firefox/148.toml"));
        reg.load_toml(include_str!("../../profiles/firefox/149.toml"));
        reg.load_toml(include_str!("../../profiles/firefox/150.toml"));
        reg.load_toml(include_str!("../../profiles/firefox/151.toml"));
        reg.load_toml(include_str!("../../profiles/firefox/152.toml"));
        reg.load_toml(include_str!("../../profiles/firefox/153.toml"));
        reg.load_toml(include_str!("../../profiles/firefox/154.toml"));
        reg.load_toml(include_str!("../../profiles/safari/18.toml"));
        reg.load_toml(include_str!("../../profiles/safari/26.toml"));
        reg.load_toml(include_str!("../../profiles/safari/ios17.toml"));
        reg.load_toml(include_str!("../../profiles/safari/ios18.toml"));
        reg.load_toml(include_str!("../../profiles/okhttp/android10.toml"));
        reg.load_toml(include_str!("../../profiles/cfnetwork/ios18.toml"));
        reg.load_toml(include_str!("../../profiles/cfnetwork/macos26.toml"));
        reg
    }

    /// Parse and insert a TOML profile string.
    fn load_toml(&mut self, toml_str: &str) {
        let profile = BrowserProfile::from_toml(toml_str)
            .expect("built-in profile is statically valid (profile_validation)");
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
mod tests;
