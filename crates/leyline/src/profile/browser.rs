use crate::profile::{BrowserHeaders, BrowserProfile, ChromiumBrand, Platform};
use crate::{Cause, Error};

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
    base: Browser,
    brand: Option<ChromiumBrand>,
}

impl Family {
    pub(crate) fn samesite_checks_redirect_chain(self) -> bool {
        FAMILY_SAMESITE_REDIRECT_CHAIN[self as usize]
    }
}

impl std::fmt::Display for Family {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(FAMILY_LABELS[*self as usize])
    }
}

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
    Error::new(Cause::Config).with_message(format!(
        "unknown {kind} {id:?}; expected one of {}",
        known.collect::<Vec<_>>().join(", ")
    ))
}

impl Browser {
    /// Returns every bundled browser, including deprecated versions.
    pub fn all() -> &'static [Browser] {
        ALL
    }

    /// Returns the browser's stable id, such as `"chrome-148"`.
    #[must_use]
    pub fn id(&self) -> &'static str {
        self.entry().id
    }

    pub(crate) fn profile_digest(self) -> u64 {
        self.entry().digest
    }

    pub(crate) fn base(self) -> Self {
        self.entry().base
    }

    pub(crate) fn brand(self) -> Option<ChromiumBrand> {
        self.entry().brand
    }

    pub(crate) fn branded(self, brand: ChromiumBrand) -> Self {
        let base = self.base();
        ALL.iter()
            .copied()
            .find(|browser| browser.base() == base && browser.brand() == Some(brand))
            .unwrap_or(base)
    }

    fn entry(self) -> &'static Entry {
        &ENTRIES[self as usize]
    }

    pub(crate) fn profile_key(&self) -> (&'static str, u32) {
        let entry = self.base().entry();
        (entry.key, entry.version)
    }

    pub(crate) fn latest(family: Family) -> Self {
        FAMILY_LATEST[family as usize].1
    }

    pub(crate) fn family(&self) -> Family {
        self.entry().family
    }

    /// Returns the browser's major version.
    #[must_use]
    pub fn version(&self) -> u32 {
        self.entry().version
    }

    #[must_use]
    pub(crate) fn hello_rep(self) -> Self {
        self.entry().hello
    }

    /// Returns the next distinct TLS ClientHello in the same family, or `None` when the family has only one.
    #[must_use]
    pub fn next_hello(self) -> Option<Self> {
        let family = self.base().family();
        let mut seen = std::collections::HashSet::new();
        let hellos: Vec<Self> = Self::all()
            .iter()
            .filter(|candidate| candidate.family() == family)
            .map(|candidate| candidate.hello_rep())
            .filter(|rep| seen.insert(*rep))
            .collect();
        if hellos.len() < 2 {
            return None;
        }
        let current = self.base().hello_rep();
        let index = hellos.iter().position(|&h| h == current).unwrap_or(0);
        Some(hellos[(index + 1) % hellos.len()])
    }

    /// Returns the variant of this browser for `platform`, or the browser itself when it has none.
    #[must_use]
    pub fn for_platform(self, platform: Platform) -> Self {
        if self.brand().is_some() {
            return self;
        }
        let key = platform.resolve().identity_key();
        self.entry()
            .platforms
            .iter()
            .find(|(name, _)| *name == key)
            .map_or(self, |&(_, browser)| browser)
    }

    /// Returns the `user-agent`, `sec-ch-ua`, and `accept-language` headers the browser sends on `platform`.
    #[must_use]
    pub fn headers(self, platform: Platform) -> Option<Vec<(&'static str, String)>> {
        let sent = self.identity_headers(platform)?;
        let mut headers = vec![("user-agent", sent.user_agent)];
        if !sent.sec_ch_ua.is_empty() {
            headers.push(("sec-ch-ua", sent.sec_ch_ua));
        }
        headers.extend(sent.accept_language.map(|value| ("accept-language", value)));
        Some(headers)
    }

    pub(crate) fn identity_headers(self, platform: Platform) -> Option<BrowserHeaders> {
        self.platform_profile(platform)
            .resolve_identity(platform, self.brand().unwrap_or_default())
            .ok()
    }

    pub(crate) fn platform_profile(self, platform: Platform) -> &'static BrowserProfile {
        self.base().for_platform(platform).profile()
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

    /// Returns the Chromium major version this browser is built on, or `None` for a non-Chromium browser.
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
