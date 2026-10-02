use crate::profile::{BrowserProfile, ChromiumBrand, Platform, PlatformIdentity, resolve_identity};
use crate::{Error, Kind};

mod digest;

pub(crate) use digest::digest;

include!(concat!(env!("OUT_DIR"), "/browser.rs"));

struct Entry {
    family: Family,
    key: &'static str,
    version: u32,
    id: &'static str,
    name: &'static str,
    hello: Browser,
    platforms: &'static [(&'static str, Browser)],
    digest: u64,
    source: &'static str,
}

impl Family {
    #[must_use]
    pub fn all() -> &'static [Family] {
        FAMILY_ALL
    }

    #[must_use]
    pub fn id(&self) -> &'static str {
        FAMILY_IDS[*self as usize]
    }

    pub(crate) fn samesite_checks_redirect_chain(self) -> bool {
        FAMILY_SAMESITE_REDIRECT_CHAIN[self as usize]
    }
}

impl std::fmt::Display for Family {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(FAMILY_LABELS[*self as usize])
    }
}

impl std::str::FromStr for Family {
    type Err = Error;

    fn from_str(id: &str) -> Result<Self, Self::Err> {
        FAMILY_ALL
            .iter()
            .copied()
            .find(|family| family.id().eq_ignore_ascii_case(id))
            .ok_or_else(|| unknown("family", id, FAMILY_IDS.iter().copied()))
    }
}

string_id_serde!(Family);

impl std::str::FromStr for Browser {
    type Err = Error;

    fn from_str(id: &str) -> Result<Self, Self::Err> {
        ALL.iter()
            .copied()
            .find(|browser| browser.id().eq_ignore_ascii_case(id))
            .ok_or_else(|| unknown("browser", id, ALL.iter().map(Browser::id)))
    }
}

string_id_serde!(Browser);

pub(crate) fn unknown<'a>(kind: &str, id: &str, known: impl Iterator<Item = &'a str>) -> Error {
    Error::new(Kind::Config).with_message(format!(
        "unknown {kind} {id:?}; expected one of {}",
        known.collect::<Vec<_>>().join(", ")
    ))
}

impl Browser {
    pub fn all() -> &'static [Browser] {
        ALL
    }

    #[must_use]
    pub fn id(&self) -> &'static str {
        self.entry().id
    }

    pub(crate) fn profile_digest(self) -> u64 {
        self.entry().digest
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
        let key = platform.resolve().identity_key();
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
        resolve_identity(
            self.platform_profile(platform),
            platform,
            brand.unwrap_or_default(),
        )
        .ok()
    }

    pub(crate) fn platform_profile(self, platform: Platform) -> &'static BrowserProfile {
        self.for_platform(platform).profile()
    }

    pub(crate) fn profile_source(self) -> &'static str {
        self.entry().source
    }

    pub(crate) fn matching_source(text: &str) -> Option<Self> {
        let wanted = digest(&[text.as_bytes()]);
        ALL.iter()
            .copied()
            .find(|browser| digest(&[browser.profile_source().as_bytes()]) == wanted)
    }

    pub(crate) fn default_browser() -> Self {
        Self::latest(DEFAULT_FAMILY)
    }

    #[cfg(feature = "bench-internals")]
    pub fn chromium_major(&self) -> Option<u32> {
        self.profile().meta.chromium_major
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
