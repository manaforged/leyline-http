use super::Session;
use crate::profile::ChromiumBrand;

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

    pub(crate) fn protocol_policy(&self) -> crate::core::Protocol {
        self.inner.protocol_policy
    }
}
