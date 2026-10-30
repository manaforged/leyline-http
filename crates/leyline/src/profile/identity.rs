use crate::profile::{BrowserHeaders, BrowserProfile, ChromiumBrand, Platform};

impl BrowserProfile {
    pub(crate) fn resolve_identity(
        &self,
        platform: Platform,
        brand: ChromiumBrand,
    ) -> Result<BrowserHeaders, String> {
        let mut identity = self
            .identity_for(platform)
            .ok_or_else(|| format!("no {platform} identity for {}", self.meta.name))?
            .clone();
        if identity.accept_language.is_none() {
            identity.accept_language = BrowserProfile::bare_shared()
                .identity_for(platform)
                .and_then(|bare| bare.accept_language.clone());
        }
        if brand == ChromiumBrand::Chrome {
            return Ok(identity);
        }
        let chromium_major = self.meta.chromium_major.ok_or_else(|| {
            format!(
                "{} overlay requires a Chromium HTTP identity",
                brand.label()
            )
        })?;
        brand
            .apply(chromium_major, platform, &mut identity)
            .map_err(|error| error.to_string())?;
        Ok(identity)
    }
}
