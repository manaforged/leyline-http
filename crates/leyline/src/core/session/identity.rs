use crate::core::error::{Error, Kind, Result};
use crate::profile::browser::digest;
use crate::profile::{Browser, ChromiumBrand, Family, Platform};

use super::builder::derive::{DerivedIdentity, IdentityInputs, IdentitySource, derive_identity};
use super::{Session, SessionInner};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "SavedIdentity", into = "SavedIdentity")]
pub struct Identity {
    http: Browser,
    tls: Browser,
    platform: Platform,
    brand: Option<ChromiumBrand>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedIdentity {
    http: Browser,
    tls: Browser,
    platform: Platform,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    brand: Option<ChromiumBrand>,
}

impl From<Identity> for SavedIdentity {
    fn from(id: Identity) -> Self {
        Self {
            http: id.http,
            tls: id.tls,
            platform: id.platform,
            brand: id.brand,
        }
    }
}

impl TryFrom<SavedIdentity> for Identity {
    type Error = Error;

    fn try_from(saved: SavedIdentity) -> Result<Self> {
        if saved.http.family() != saved.tls.family() {
            return Err(Error::new(Kind::Config).with_message(format!(
                "saved identity tls {} is not the same family as {}",
                saved.tls, saved.http
            )));
        }
        Ok(Self {
            http: saved.http,
            tls: saved.tls,
            platform: saved.platform,
            brand: saved.brand,
        })
    }
}

impl Identity {
    #[must_use]
    pub fn locked(browser: Browser, platform: Platform) -> Self {
        Self {
            http: browser,
            tls: browser.hello_rep(),
            platform,
            brand: None,
        }
    }

    #[must_use]
    pub fn with_brand(self, brand: ChromiumBrand) -> Self {
        Self {
            brand: Some(brand),
            ..self
        }
    }

    #[must_use]
    pub fn brand(self) -> Option<ChromiumBrand> {
        self.brand
    }

    pub fn rotate_tls(self, tls: Browser) -> Result<Self> {
        if self.http.family() != tls.family() {
            return Err(Error::new(Kind::Config).with_message(format!(
                "tls rotate {tls} is not the same family as {}",
                self.http
            )));
        }
        Ok(Self {
            tls: tls.hello_rep(),
            ..self
        })
    }

    pub fn rotate_hello(self) -> Result<Self> {
        let mut hellos: Vec<Browser> = Vec::new();
        for candidate in Browser::all() {
            if candidate.family() != self.http.family() {
                continue;
            }
            let rep = candidate.hello_rep();
            if !hellos.contains(&rep) {
                hellos.push(rep);
            }
        }
        if hellos.len() < 2 {
            return Err(Error::new(Kind::Config)
                .with_message(format!("no other hello in family {}", self.http.family())));
        }
        let current = self.tls.hello_rep();
        let index = hellos.iter().position(|&h| h == current).unwrap_or(0);
        self.rotate_tls(hellos[(index + 1) % hellos.len()])
    }

    pub fn switch_family(self, dest: Browser) -> Result<Self> {
        if dest.family() == self.http.family() {
            return Err(Error::new(Kind::Config).with_message(format!(
                "switch_family {dest} is the same family as {} — rotate_tls instead",
                self.http
            )));
        }
        let id = Self::locked(dest, self.platform);
        drop(id.user_agent()?);
        Ok(id)
    }

    #[must_use]
    pub fn http(self) -> Browser {
        self.http
    }

    #[must_use]
    pub fn tls(self) -> Browser {
        self.tls
    }

    #[must_use]
    pub fn platform(self) -> Platform {
        self.platform
    }

    pub(crate) fn user_agent(self) -> Result<String> {
        self.http
            .platform_profile(self.platform)
            .resolve_presented(self.platform, ChromiumBrand::Chrome)
            .map(|identity| identity.user_agent)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SessionIdentity {
    identity: Option<Identity>,
    platform: Platform,
    brand: Option<ChromiumBrand>,
    user_agent: String,
    saved: Option<Identity>,
    profile_id: Option<String>,
    profile_toml: Option<std::sync::Arc<str>>,
    languages: Option<Vec<String>>,
}

impl SessionIdentity {
    #[must_use]
    pub fn to_identity(&self) -> Option<Identity> {
        self.saved
    }

    #[must_use]
    pub fn profile_id(&self) -> Option<&str> {
        self.profile_id.as_deref()
    }

    #[must_use]
    pub fn identity(&self) -> Option<Identity> {
        self.identity
    }

    #[must_use]
    pub fn export_profile(&self) -> Option<String> {
        self.profile_toml.as_deref().map(str::to_owned)
    }

    #[must_use]
    pub fn languages(&self) -> Option<&[String]> {
        self.languages.as_deref()
    }

    #[must_use]
    pub fn browser(&self) -> Option<Browser> {
        self.identity.map(Identity::http)
    }

    #[must_use]
    pub fn platform(&self) -> Platform {
        self.platform
    }

    #[must_use]
    pub fn brand(&self) -> Option<ChromiumBrand> {
        self.brand
    }

    #[must_use]
    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }
}

impl Session {
    #[must_use]
    pub fn identity(&self) -> SessionIdentity {
        let brand = match self.inner.brand {
            ChromiumBrand::Chrome => self
                .inner
                .browser
                .filter(|browser| browser.family() == Family::Chrome)
                .map(|_| ChromiumBrand::Chrome),
            other => Some(other),
        };
        let saved = self.saved_identity(brand);
        let profile_toml = self
            .inner
            .impersonates
            .then(|| self.inner.profile.source())
            .flatten()
            .map(std::sync::Arc::from);
        let profile_id = match (saved, profile_toml.as_deref()) {
            (Some(id), _) => Some(profile_id(id, self.inner.brand)),
            (None, Some(source)) => Some(source_profile_id(
                source,
                self.inner.platform,
                self.inner.brand,
            )),
            (None, None) => None,
        };
        SessionIdentity {
            identity: self.inner.identity,
            platform: self.inner.platform,
            brand,
            user_agent: self.inner.user_agent.clone(),
            saved,
            profile_id,
            profile_toml,
            languages: self.inner.languages.clone(),
        }
    }

    fn saved_identity(&self, brand: Option<ChromiumBrand>) -> Option<Identity> {
        let identity = self.inner.identity?;
        Some(Identity {
            tls: self.inner.browser?,
            brand,
            ..identity
        })
    }

    pub fn with_identity(&self, identity: Identity) -> Result<Self> {
        if self.inner.browser.is_none() {
            return Err(Error::new(Kind::Config)
                .with_message("with_identity requires an impersonating session"));
        }
        let mut session = self.clone();
        let inner = std::sync::Arc::make_mut(&mut session.inner);
        let brand = identity.brand().unwrap_or(inner.brand);
        let derived = derive_identity(IdentityInputs {
            source: IdentitySource::Browser {
                tls: identity.tls(),
                http: identity.http(),
            },
            platform: identity.platform(),
            brand,
            compression: inner.compression,
            #[cfg(feature = "http3")]
            h3_required: inner.protocol_policy.requires_h3(),
            tcp: inner.connector.tcp_profile(),
            audit: inner.audit_tls.is_some(),
            default_headers: &inner.default_headers,
            languages: inner.languages.as_deref(),
        })?;
        inner.connector = inner
            .connector
            .with_profile(&derived.profile)
            .map_err(Error::from)?;
        inner.pool = std::sync::Arc::new(inner.pool.shared_view(profile_hash(identity, brand)));
        inner.adopt(derived);
        inner.brand = brand;
        Ok(session)
    }
}

fn profile_id(identity: Identity, brand: ChromiumBrand) -> String {
    let hash = profile_hash(identity, brand);
    format!("{hash:016x}")
}

fn profile_hash(identity: Identity, brand: ChromiumBrand) -> u64 {
    let http = identity.http.profile_digest().to_le_bytes();
    let tls = identity.tls.profile_digest().to_le_bytes();
    digest(&[
        &http,
        &tls,
        identity.platform.id().as_bytes(),
        brand.id().as_bytes(),
    ])
}

fn source_profile_id(source: &str, platform: Platform, brand: ChromiumBrand) -> String {
    match Browser::matching_source(source) {
        Some(browser) => profile_id(
            Identity {
                http: browser,
                tls: browser,
                platform,
                brand: None,
            },
            brand,
        ),
        None => format!(
            "{:016x}",
            digest(&[
                source.as_bytes(),
                platform.id().as_bytes(),
                brand.id().as_bytes()
            ])
        ),
    }
}

impl SessionInner {
    fn adopt(&mut self, derived: DerivedIdentity) {
        let DerivedIdentity {
            browser,
            identity,
            platform,
            profile,
            header_style,
            header_order,
            user_agent,
            sec_ch_ua,
            accept_language,
            h2_config,
            #[cfg(feature = "http3")]
            h3_config,
            audit_tls,
        } = derived;
        self.browser = browser;
        self.identity = identity;
        self.platform = platform;
        self.profile = profile;
        self.header_style = header_style;
        self.header_order = header_order;
        self.user_agent = user_agent;
        self.sec_ch_ua = sec_ch_ua;
        self.accept_language = accept_language;
        self.h2_config = h2_config;
        #[cfg(feature = "http3")]
        {
            self.h3_config = h3_config;
        }
        self.audit_tls = audit_tls;
    }
}
