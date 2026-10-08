use crate::profile::brand::BrandOverlayError;
use crate::profile::{BrowserProfile, ChromiumBrand, Platform, PlatformIdentity};

#[derive(Debug)]
pub(crate) enum IdentityError {
    NoPlatform(String, Platform),
    NotChromium(ChromiumBrand),
    Brand(BrandOverlayError),
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPlatform(label, platform) => write!(f, "no {platform} identity for {label}"),
            Self::NotChromium(brand) => write!(
                f,
                "{} overlay requires a Chromium HTTP identity",
                brand.label()
            ),
            Self::Brand(error) => std::fmt::Display::fmt(error, f),
        }
    }
}

impl BrowserProfile {
    pub(crate) fn resolve_identity(
        &self,
        platform: Platform,
        brand: ChromiumBrand,
    ) -> Result<PlatformIdentity, IdentityError> {
        let mut identity = self
            .identity_for(platform)
            .ok_or_else(|| IdentityError::NoPlatform(self.meta.name.clone(), platform))?
            .clone();
        if identity.accept_language.is_none() {
            identity.accept_language = BrowserProfile::bare_shared()
                .identity_for(platform)
                .and_then(|bare| bare.accept_language.clone());
        }
        if brand == ChromiumBrand::Chrome {
            return Ok(identity);
        }
        let chromium_major = self
            .meta
            .chromium_major
            .ok_or(IdentityError::NotChromium(brand))?;
        brand
            .apply(chromium_major, platform, &mut identity)
            .map_err(IdentityError::Brand)?;
        Ok(identity)
    }
}
