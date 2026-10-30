use std::sync::Arc;

use crate::core::error::{Cause, Error, Result};
use crate::profile::browser::digest;
use crate::profile::{Browser, BrowserProfile, ChromiumBrand, Family, Platform};

use super::{Session, SessionInner};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "SavedIdentity", into = "SavedIdentity")]
pub(crate) struct Identity {
    source: Source,
    platform: Platform,
    brand: Option<ChromiumBrand>,
    languages: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub(super) enum Source {
    Plain,
    Browser { http: Browser, tls: Browser },
    Profile(Arc<BrowserProfile>),
}

impl PartialEq for Source {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Plain, Self::Plain) => true,
            (Self::Browser { http, tls }, Self::Browser { http: h, tls: t }) => {
                http == h && tls == t
            }
            (Self::Profile(a), Self::Profile(b)) => {
                Arc::ptr_eq(a, b) || a.source().is_some() && a.source() == b.source()
            }
            _ => false,
        }
    }
}

impl Eq for Source {}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    http: Option<Browser>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tls: Option<Browser>,
    platform: Platform,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    brand: Option<ChromiumBrand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    languages: Option<Vec<String>>,
}

impl From<Identity> for SavedIdentity {
    fn from(id: Identity) -> Self {
        let (http, tls, profile) = match id.source {
            Source::Plain => (None, None, None),
            Source::Browser { http, tls } => (Some(http), Some(tls), None),
            Source::Profile(profile) => (None, None, profile.source().map(str::to_owned)),
        };
        Self {
            http,
            tls,
            platform: id.platform,
            brand: id.brand,
            profile,
            languages: id.languages,
        }
    }
}

impl TryFrom<SavedIdentity> for Identity {
    type Error = Error;

    fn try_from(saved: SavedIdentity) -> Result<Self> {
        let source = match (saved.http, saved.tls, saved.profile) {
            (Some(http), Some(tls), None) => {
                if http.family() != tls.family() {
                    return Err(Error::new(Cause::Config).with_message(format!(
                        "saved identity tls {tls} is not the same family as {http}"
                    )));
                }
                Source::Browser { http, tls }
            }
            (None, None, Some(toml)) => {
                Source::Profile(Arc::new(BrowserProfile::from_toml(&toml)?))
            }
            (None, None, None) => Source::Plain,
            _ => {
                return Err(Error::new(Cause::Config).with_message(
                    "saved identity needs both http and tls, or a profile, or neither",
                ));
            }
        };
        Ok(Self {
            source,
            platform: saved.platform,
            brand: saved.brand,
            languages: saved.languages,
        })
    }
}

impl From<Browser> for Identity {
    fn from(browser: Browser) -> Self {
        let platform = Platform::all()
            .iter()
            .copied()
            .find(|&platform| {
                platform != Platform::Host
                    && browser.for_platform(platform) == browser
                    && browser.headers(platform).is_some()
            })
            .unwrap_or(Platform::Windows);
        Self::new(browser, platform)
    }
}

impl Identity {
    #[must_use]
    pub(crate) fn new(browser: Browser, platform: Platform) -> Self {
        let base = browser.base();
        Self {
            brand: browser.brand(),
            ..Self::with_source(
                Source::Browser {
                    http: base,
                    tls: base,
                },
                platform,
            )
        }
    }

    #[must_use]
    pub(crate) fn from_profile(profile: BrowserProfile, platform: Platform) -> Self {
        Self::with_source(Source::Profile(Arc::new(profile)), platform)
    }

    #[must_use]
    pub(crate) fn plain(platform: Platform) -> Self {
        Self::with_source(Source::Plain, platform)
    }

    fn with_source(source: Source, platform: Platform) -> Self {
        Self {
            source,
            platform,
            brand: None,
            languages: None,
        }
    }

    #[must_use]
    pub(crate) fn with_brand(self, brand: ChromiumBrand) -> Self {
        Self {
            brand: Some(brand),
            ..self
        }
    }

    #[must_use]
    pub(crate) fn with_languages<I, S>(self, languages: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let languages = languages
            .into_iter()
            .map(|tag| tag.as_ref().to_owned())
            .collect();
        Self {
            languages: Some(languages),
            ..self
        }
    }

    pub(crate) fn rotate_tls(self, tls: Browser) -> Result<Self> {
        let tls = tls.base();
        let Source::Browser { http, .. } = self.source else {
            return Err(Error::new(Cause::Config)
                .with_message("tls rotate needs a browser identity, not a profile or plain one"));
        };
        if http.family() != tls.family() {
            return Err(Error::new(Cause::Config)
                .with_message(format!("tls rotate {tls} is not the same family as {http}")));
        }
        Ok(Self {
            source: Source::Browser {
                http,
                tls: tls.hello_rep(),
            },
            ..self
        })
    }

    #[must_use]
    pub(crate) fn http(&self) -> Option<Browser> {
        match self.source {
            Source::Browser { http, .. } => {
                Some(self.brand.map_or(http, |brand| http.branded(brand)))
            }
            _ => None,
        }
    }

    #[must_use]
    pub(crate) fn tls(&self) -> Option<Browser> {
        match self.source {
            Source::Browser { tls, .. } => Some(tls),
            _ => None,
        }
    }

    #[must_use]
    pub(crate) fn platform(&self) -> Platform {
        self.platform
    }

    #[cfg(test)]
    pub(crate) fn brand(&self) -> Option<ChromiumBrand> {
        self.brand
    }

    pub(super) fn into_parts(
        self,
    ) -> (Source, Platform, Option<ChromiumBrand>, Option<Vec<String>>) {
        (self.source, self.platform, self.brand, self.languages)
    }
}

impl Session {
    /// Returns the browser the session sends HTTP headers as.
    #[must_use]
    pub fn browser(&self) -> Option<Browser> {
        self.identity().http()
    }

    /// Returns the browser whose TLS ClientHello the session sends.
    #[must_use]
    pub fn hello(&self) -> Option<Browser> {
        self.identity().tls()
    }

    /// Returns the operating system the session reports.
    #[must_use]
    pub fn platform(&self) -> Platform {
        self.inner.platform
    }

    /// Returns the language tags set on the session, if any.
    #[must_use]
    pub fn languages(&self) -> Option<&[String]> {
        self.inner.languages.as_deref()
    }

    #[must_use]
    pub(crate) fn identity(&self) -> Identity {
        let inner = &self.inner;
        let mut identity = match (inner.saved_identity(), inner.inputs.loaded_profile()) {
            (Some(saved), _) => saved,
            (None, Some(profile)) => {
                Identity::with_source(Source::Profile(Arc::clone(profile)), inner.platform)
            }
            (None, None) => Identity::plain(inner.platform),
        };
        identity.brand = inner.shown_brand();
        identity.languages = inner.languages.clone();
        identity
    }

    /// Returns the `User-Agent` header value the session sends.
    #[must_use]
    pub fn user_agent(&self) -> &str {
        &self.inner.user_agent
    }

    /// Returns the profile hash as 16 hex digits when the session has a profile.
    #[must_use]
    pub fn profile_id(&self) -> Option<String> {
        self.inner.profile_hash().map(|hash| format!("{hash:016x}"))
    }

    /// Returns the TOML source of the session's base profile, before any brand overlay, when it has one.
    #[must_use]
    pub fn profile(&self) -> Option<&str> {
        self.inner.profile_source()
    }

    pub(crate) fn http_browser(&self) -> Option<Browser> {
        self.inner.identity.as_ref().and_then(Identity::http)
    }
}

impl SessionInner {
    fn shown_brand(&self) -> Option<ChromiumBrand> {
        match self.brand {
            ChromiumBrand::Chrome => self
                .browser
                .filter(|browser| browser.family() == Family::Chrome)
                .map(|_| ChromiumBrand::Chrome),
            other => Some(other),
        }
    }

    fn saved_identity(&self) -> Option<Identity> {
        let http = self.identity.as_ref()?.http()?.base();
        Some(Identity {
            source: Source::Browser {
                http,
                tls: self.browser?,
            },
            platform: self.identity.as_ref()?.platform(),
            brand: self.shown_brand(),
            languages: None,
        })
    }

    fn profile_source(&self) -> Option<&str> {
        self.impersonates.then(|| self.profile.source()).flatten()
    }

    pub(super) fn profile_hash(&self) -> Option<u64> {
        match (self.saved_identity(), self.profile_source()) {
            (Some(id), _) => Some(profile_hash(
                id.http()?.base(),
                id.tls()?,
                id.platform,
                self.brand,
            )),
            (None, Some(source)) => Some(source_profile_hash(source, self.platform, self.brand)),
            (None, None) => None,
        }
    }
}

fn profile_hash(http: Browser, tls: Browser, platform: Platform, brand: ChromiumBrand) -> u64 {
    let http = http.profile_digest().to_le_bytes();
    let tls = tls.profile_digest().to_le_bytes();
    digest(&[&http, &tls, platform.id().as_bytes(), brand.id().as_bytes()])
}

fn source_profile_hash(source: &str, platform: Platform, brand: ChromiumBrand) -> u64 {
    match Browser::matching_source(source) {
        Some(browser) => profile_hash(browser, browser, platform, brand),
        None => digest(&[
            source.as_bytes(),
            platform.id().as_bytes(),
            brand.id().as_bytes(),
        ]),
    }
}
