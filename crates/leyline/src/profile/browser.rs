use crate::profile::preset::HeaderStyle;
use crate::profile::{
    ChromiumBrand, Platform, PlatformIdentity, ProfileRegistry, resolve_identity,
};

include!(concat!(env!("OUT_DIR"), "/browser.rs"));

struct Entry {
    family: Family,
    key: &'static str,
    version: u32,
    name: &'static str,
    hello: Browser,
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

    pub(crate) fn profile_key(&self) -> (&'static str, u32) {
        let entry = self.entry();
        (entry.key, entry.version)
    }

    #[must_use]
    pub fn get(family: Family, version: u32) -> Option<Self> {
        ALL.iter().copied().find(|browser| {
            let entry = browser.entry();
            entry.family == family && entry.version == version
        })
    }

    #[must_use]
    pub fn latest(family: Family) -> Self {
        FAMILY_LATEST[family as usize]
    }

    #[must_use]
    pub fn family(&self) -> Family {
        self.entry().family
    }

    #[must_use]
    pub fn version(&self) -> u32 {
        self.entry().version
    }

    #[must_use]
    pub(crate) fn hello_rep(self) -> Self {
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
    pub fn identity(
        self,
        platform: Platform,
        brand: Option<ChromiumBrand>,
    ) -> Option<PlatformIdentity> {
        let resolved = resolve_identity(Some(self), platform, brand.unwrap_or_default()).ok()?;
        let mut identity = resolved.identity;
        identity.extra_headers.extend(resolved.brand_extra_headers);
        if resolved.brand_navigate_accept.is_some() {
            identity.navigate_accept_override = resolved.brand_navigate_accept;
        }
        Some(identity)
    }

    pub(crate) fn header_style(self) -> HeaderStyle {
        ProfileRegistry::global()
            .get_browser(self)
            .map_or_else(HeaderStyle::default, |profile| profile.meta.header_style)
    }

    pub(crate) fn profile_source(self) -> &'static str {
        self.entry().source
    }

    pub(crate) fn default_browser() -> Self {
        Self::latest(DEFAULT_FAMILY)
    }

    bench_pub! {
        fn chromium_major(&self) -> Option<u32> {
            self.entry().chromium_major
        }
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
