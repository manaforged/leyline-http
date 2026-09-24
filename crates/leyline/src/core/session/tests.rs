use super::{Identity, Session};
use crate::profile::{ChromiumBrand, Platform};

mod bare;
mod brand;
mod brand_config;
mod cookie;
mod h1;
mod h1_stream;
mod identity;
mod proxy;
mod redirect;

#[cfg(target_os = "linux")]
mod tcp;

impl Session {
    pub(crate) fn identity(&self) -> Option<Identity> {
        self.inner.identity
    }

    pub(crate) fn brand(&self) -> Option<ChromiumBrand> {
        match self.inner.brand {
            ChromiumBrand::Chrome => match self.inner.browser {
                Some(browser) if browser.family() == crate::profile::Family::Chrome => {
                    Some(ChromiumBrand::Chrome)
                }
                _ => None,
            },
            other => Some(other),
        }
    }

    pub(crate) fn platform(&self) -> Platform {
        self.inner.platform
    }

    pub(crate) fn protocol_policy(&self) -> crate::core::ProtocolPolicy {
        self.inner.protocol_policy
    }
}
