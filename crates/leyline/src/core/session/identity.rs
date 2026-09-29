use crate::core::error::{Error, Kind, Result};
use crate::profile::{Browser, ChromiumBrand, Family, Platform};

use super::builder::derive::{
    DerivedIdentity, IdentityInputs, IdentitySource, derive_identity, resolve_presented,
};
use super::{Session, SessionInner};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Identity {
    http: Browser,
    tls: Browser,
    platform: Platform,
}

impl Identity {
    #[must_use]
    pub fn locked(browser: Browser, platform: Platform) -> Self {
        Self {
            http: browser,
            tls: browser.hello_rep(),
            platform,
        }
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
        resolve_presented(
            self.http.platform_profile(self.platform),
            self.platform,
            ChromiumBrand::Chrome,
        )
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
}

impl SessionIdentity {
    #[must_use]
    pub fn identity(&self) -> Option<Identity> {
        self.identity
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
        SessionIdentity {
            identity: self.inner.identity,
            platform: self.inner.platform,
            brand,
            user_agent: self.inner.user_agent.clone(),
        }
    }

    pub fn with_identity(&self, identity: Identity) -> Result<Self> {
        if self.inner.browser.is_none() {
            return Err(Error::new(Kind::Config)
                .with_message("with_identity requires an impersonating session"));
        }
        let mut session = self.clone();
        let inner = std::sync::Arc::make_mut(&mut session.inner);
        let derived = derive_identity(IdentityInputs {
            source: IdentitySource::Browser {
                tls: identity.tls(),
                http: identity.http(),
            },
            platform: identity.platform(),
            brand: inner.brand,
            compression: inner.compression,
            #[cfg(feature = "http3")]
            h3_required: inner.protocol_policy.requires_h3(),
            tcp: inner.connector.tcp_profile(),
            audit: inner.audit_tls.is_some(),
        })?;
        inner.connector = inner
            .connector
            .with_profile(&derived.profile)
            .map_err(Error::from)?;
        inner.pool = std::sync::Arc::new(inner.pool.fresh());
        inner.adopt(derived);
        Ok(session)
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
