use std::sync::{Arc, LazyLock};

use crate::profile::Browser;
use crate::profile::types::BrowserProfile;

pub struct ProfileRegistry {
    profiles: Vec<Arc<BrowserProfile>>,
}

impl std::fmt::Debug for ProfileRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut keys: Vec<(&str, u32)> = self
            .profiles
            .iter()
            .map(|profile| (profile.meta.browser.as_str(), profile.meta.version))
            .collect();
        keys.sort_unstable();
        f.debug_struct("ProfileRegistry")
            .field("profiles", &keys)
            .finish_non_exhaustive()
    }
}

static BUILTIN: LazyLock<ProfileRegistry> = LazyLock::new(ProfileRegistry::builtin);

impl ProfileRegistry {
    #[must_use]
    pub(crate) fn global() -> &'static Self {
        &BUILTIN
    }

    fn new() -> Self {
        Self {
            profiles: Vec::new(),
        }
    }

    bench_pub! {
        fn builtin() -> Self {
            let mut reg = Self::new();
            for browser in Browser::all() {
                reg.load_toml(browser.profile_source());
            }
            reg
        }
    }

    fn load_toml(&mut self, toml_str: &str) {
        let profile = BrowserProfile::from_toml(toml_str)
            .expect("built-in profile is statically valid (profile_validation)");
        self.insert(profile);
    }

    fn insert(&mut self, profile: BrowserProfile) {
        let profile = Arc::new(profile);
        match self.profiles.iter_mut().find(|old| {
            old.meta.browser == profile.meta.browser && old.meta.version == profile.meta.version
        }) {
            Some(slot) => *slot = profile,
            None => self.profiles.push(profile),
        }
    }

    fn find(&self, browser: &str, version: u32) -> Option<&Arc<BrowserProfile>> {
        self.profiles
            .iter()
            .find(|profile| profile.meta.browser == browser && profile.meta.version == version)
    }

    pub(crate) fn get(&self, browser: &str, version: u32) -> Option<&BrowserProfile> {
        self.find(browser, version).map(|profile| &**profile)
    }

    pub(crate) fn shared(&self, browser: Browser) -> Option<Arc<BrowserProfile>> {
        let (name, version) = browser.profile_key();
        self.find(name, version).cloned()
    }

    bench_pub! {
        fn get_browser(&self, browser: Browser) -> Option<&BrowserProfile> {
            let (name, version) = browser.profile_key();
            self.get(name, version)
        }
    }
}

#[cfg(test)]
mod tests;
