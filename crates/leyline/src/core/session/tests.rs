use super::Session;
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
    pub(crate) fn brand(&self) -> Option<ChromiumBrand> {
        self.identity().brand()
    }

    pub(crate) fn platform(&self) -> Platform {
        self.inner.platform
    }

    pub(crate) fn protocol_policy(&self) -> crate::core::ProtocolPolicy {
        self.inner.protocol_policy
    }
}
