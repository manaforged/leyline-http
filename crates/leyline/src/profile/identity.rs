use crate::profile::brand::BrandOverlayError;
use crate::profile::{BrowserProfile, ChromiumBrand, Platform, PlatformIdentity};

pub(crate) struct ResolvedIdentity {
    pub(crate) identity: PlatformIdentity,
    pub(crate) brand_extra_headers: Vec<(String, String)>,
    pub(crate) brand_navigate_accept: Option<String>,
}

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

pub(crate) fn resolve_identity(
    profile: &BrowserProfile,
    platform: Platform,
    brand: ChromiumBrand,
) -> Result<ResolvedIdentity, IdentityError> {
    let mut identity = profile
        .identity_for(platform)
        .ok_or_else(|| IdentityError::NoPlatform(profile.meta.name.clone(), platform))?
        .clone();
    if brand == ChromiumBrand::Chrome {
        return Ok(ResolvedIdentity {
            identity,
            brand_extra_headers: Vec::new(),
            brand_navigate_accept: None,
        });
    }
    let chromium_major = profile
        .meta
        .chromium_major
        .ok_or(IdentityError::NotChromium(brand))?;
    let (brand_extra_headers, brand_navigate_accept) = brand
        .apply(chromium_major, platform, &mut identity)
        .map_err(IdentityError::Brand)?;
    Ok(ResolvedIdentity {
        identity,
        brand_extra_headers,
        brand_navigate_accept,
    })
}
