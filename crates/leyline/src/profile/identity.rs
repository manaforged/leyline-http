use crate::profile::brand::BrandOverlayError;
use crate::profile::{
    Browser, BrowserProfile, ChromiumBrand, Platform, PlatformIdentity, ProfileRegistry,
};

pub(crate) struct ResolvedIdentity {
    pub(crate) identity: PlatformIdentity,
    pub(crate) brand_extra_headers: Vec<(String, String)>,
    pub(crate) brand_navigate_accept: Option<String>,
}

#[derive(Debug)]
pub(crate) enum IdentityError {
    NoProfile(Browser),
    NoPlatform(String, Platform),
    NotChromium(ChromiumBrand),
    Brand(BrandOverlayError),
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoProfile(browser) => write!(f, "no profile for {browser}"),
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

fn profile_for(browser: Option<Browser>) -> Result<&'static BrowserProfile, IdentityError> {
    match browser {
        Some(browser) => ProfileRegistry::global()
            .get_browser(browser)
            .ok_or(IdentityError::NoProfile(browser)),
        None => Ok(BrowserProfile::bare_static()),
    }
}

pub(crate) fn resolve_identity(
    browser: Option<Browser>,
    platform: Platform,
    brand: ChromiumBrand,
) -> Result<ResolvedIdentity, IdentityError> {
    let twin = browser.map(|browser| browser.for_platform(platform));
    let label = twin.map_or_else(|| "bare".to_string(), |browser| browser.to_string());
    let mut identity = profile_for(twin)?
        .identity_for(platform)
        .ok_or_else(|| IdentityError::NoPlatform(label, platform))?
        .clone();
    if brand == ChromiumBrand::Chrome {
        return Ok(ResolvedIdentity {
            identity,
            brand_extra_headers: Vec::new(),
            brand_navigate_accept: None,
        });
    }
    let chromium_major = browser
        .and_then(|browser| browser.chromium_major())
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
