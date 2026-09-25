use crate::core::error::{Error, Kind, Result};
use crate::profile::{Browser, ChromiumBrand, Family, Platform, resolve_identity};

use super::Session;

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
        resolve_identity(
            self.http.platform_profile(self.platform),
            self.platform,
            ChromiumBrand::Chrome,
        )
        .map(|identity| identity.user_agent)
        .map_err(|e| Error::new(Kind::Config).with_message(e.to_string()))
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
}
