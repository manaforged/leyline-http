use crate::profile::preset::HeaderStyle;
use crate::profile::{Platform, ProfileRegistry};

include!(concat!(env!("OUT_DIR"), "/browser.rs"));

struct Entry {
    key: &'static str,
    version: u32,
    name: &'static str,
    hello: Browser,
    hellos: &'static [Browser],
    chromium_major: Option<u32>,
    platforms: &'static [(&'static str, Browser)],
    source: &'static str,
}

impl std::fmt::Display for Family {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(FAMILY_LABELS[*self as usize])
    }
}

impl Browser {
    pub fn all() -> &'static [Browser] {
        ALL
    }

    fn entry(self) -> &'static Entry {
        &ENTRIES[self as usize]
    }

    pub fn profile_key(&self) -> (&'static str, u32) {
        let entry = self.entry();
        (entry.key, entry.version)
    }

    #[must_use]
    pub fn latest(family: Family) -> Self {
        FAMILY_LATEST[family as usize]
    }

    #[must_use]
    pub fn family(&self) -> &'static str {
        self.entry().key
    }

    #[must_use]
    pub fn hello_rep(self) -> Self {
        self.entry().hello
    }

    #[must_use]
    pub fn for_platform(self, platform: Platform) -> Self {
        if platform == Platform::Host {
            return self;
        }
        let key = platform.identity_key();
        self.entry()
            .platforms
            .iter()
            .find(|(name, _)| *name == key)
            .map_or(self, |&(_, browser)| browser)
    }

    #[must_use]
    pub fn family_hellos(self) -> &'static [Self] {
        self.entry().hellos
    }

    pub(crate) fn cookie_pass_targets() -> &'static [Self] {
        COOKIE_PASS
    }

    pub(crate) fn header_style(self) -> HeaderStyle {
        ProfileRegistry::global()
            .get_browser(self)
            .map_or_else(HeaderStyle::default, |profile| profile.meta.header_style)
    }

    pub(crate) fn profile_source(self) -> &'static str {
        self.entry().source
    }

    pub fn default_browser() -> Self {
        Self::latest(DEFAULT_FAMILY)
    }

    pub fn default_firefox() -> Self {
        Self::latest(Family::Firefox)
    }

    pub fn chromium_major(&self) -> Option<u32> {
        self.entry().chromium_major
    }
}

impl Default for Browser {
    fn default() -> Self {
        Self::default_browser()
    }
}

impl std::fmt::Display for Browser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.entry().name)
    }
}

#[cfg(test)]
mod tests;
