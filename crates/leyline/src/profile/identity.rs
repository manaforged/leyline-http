use crate::error::{Error, Result};
use crate::profile::{BrowserHeaders, BrowserProfile, ChromiumBrand, Platform};

impl BrowserProfile {
    pub(crate) fn resolve_identity(
        &self,
        platform: Platform,
        brand: ChromiumBrand,
    ) -> Result<BrowserHeaders> {
        let mut identity = self
            .identity_for(platform)
            .ok_or_else(|| Error::config(format!("no {platform} identity for {}", self.meta.name)))?
            .clone();
        if identity.accept_language.is_none() {
            identity.accept_language = platform.default_accept_language().map(str::to_owned);
        }
        if brand == ChromiumBrand::Chrome {
            return Ok(identity);
        }
        let chromium_major = self.meta.chromium_major.ok_or_else(|| {
            Error::config(format!(
                "{} overlay requires a Chromium HTTP identity",
                brand.label()
            ))
        })?;
        brand
            .apply(chromium_major, platform, &mut identity)
            .map_err(|error| Error::config(error.to_string()))?;
        Ok(identity)
    }
}
