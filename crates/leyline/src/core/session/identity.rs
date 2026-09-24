use crate::core::error::{Error, Kind, Result};
use crate::profile::{Browser, ChromiumBrand, Platform, resolve_identity};

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
        resolve_identity(Some(self.http), self.platform, ChromiumBrand::Chrome)
            .map(|resolved| resolved.identity.user_agent)
            .map_err(|e| Error::new(Kind::Config).with_message(e.to_string()))
    }
}
