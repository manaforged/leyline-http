use std::collections::HashMap;
use std::sync::LazyLock;

use serde::Deserialize;

use crate::profile::{Platform, PlatformIdentity};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ChromiumBrand {
    #[default]
    Chrome,
    Edge,
    Opera,
    Vivaldi,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum BrandOverlayError {
    Unverified {
        brand: ChromiumBrand,
        chromium_major: u32,
        platform: Platform,
    },
}

impl std::fmt::Display for BrandOverlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unverified {
                brand,
                chromium_major,
                platform,
            } => write!(
                f,
                "{} overlay on Chromium {chromium_major} / {platform} is not verified \
                 against a live capture; drop the brand or pick an anchor that has one",
                brand.label(),
            ),
        }
    }
}

impl std::error::Error for BrandOverlayError {}

impl ChromiumBrand {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Chrome => "Chrome",
            Self::Edge => "Edge",
            Self::Opera => "Opera",
            Self::Vivaldi => "Vivaldi",
        }
    }

    pub(crate) fn overlay(
        self,
        chromium_major: u32,
        platform: Platform,
        profile_user_agent: &str,
    ) -> Result<Option<BrandOverlay>, BrandOverlayError> {
        let unverified = || BrandOverlayError::Unverified {
            brand: self,
            chromium_major,
            platform,
        };
        let row = brand_row(self).ok_or_else(unverified)?;
        let Some(token) = row.ua_token.as_deref() else {
            return Ok(None);
        };
        if row.desktop_only && !is_desktop(platform) {
            return Err(unverified());
        }
        let version = row
            .version(chromium_major, ua_seed(profile_user_agent))
            .ok_or_else(unverified)?;
        let product = match row.product.as_deref() {
            Some(name) => Some((name, version_major(&version).ok_or_else(unverified)?)),
            None => None,
        };
        Ok(Some(BrandOverlay {
            user_agent: format!("{profile_user_agent} {token}/{version}"),
            sec_ch_ua: sec_ch_ua(chromium_major, product),
            extra_headers: row.extra_headers.clone(),
            navigate_accept: row.navigate_accept.clone(),
        }))
    }
}

impl ChromiumBrand {
    pub(crate) fn apply(
        self,
        chromium_major: u32,
        platform: Platform,
        identity: &mut PlatformIdentity,
    ) -> Result<(Vec<(String, String)>, Option<String>), BrandOverlayError> {
        let Some(overlay) = self.overlay(chromium_major, platform, &identity.user_agent)? else {
            return Ok((Vec::new(), None));
        };
        identity.user_agent = overlay.user_agent;
        identity.sec_ch_ua = overlay.sec_ch_ua;
        Ok((overlay.extra_headers, overlay.navigate_accept))
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub(crate) struct BrandOverlay {
    pub(crate) user_agent: String,
    pub(crate) sec_ch_ua: String,
    pub(crate) extra_headers: Vec<(String, String)>,
    pub(crate) navigate_accept: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BrandRow {
    product: Option<String>,
    ua_token: Option<String>,
    #[serde(default)]
    follow_chromium: bool,
    #[serde(default)]
    desktop_only: bool,
    #[serde(default)]
    versions: HashMap<String, Vec<String>>,
    #[serde(default)]
    extra_headers: Vec<(String, String)>,
    navigate_accept: Option<String>,
}

impl BrandRow {
    fn version(&self, chromium_major: u32, seed: u64) -> Option<String> {
        if self.follow_chromium {
            return Some(format!("{chromium_major}.0.0.0"));
        }
        let builds = self.versions.get(&chromium_major.to_string())?;
        let count = u64::try_from(builds.len()).ok().filter(|n| *n > 0)?;
        builds.get(usize::try_from(seed % count).ok()?).cloned()
    }
}

static BRANDS: LazyLock<HashMap<String, BrandRow>> = LazyLock::new(|| {
    toml::from_str(include_str!("../../profiles/brands.toml"))
        .expect("built-in brand table is statically valid")
});

fn brand_row(brand: ChromiumBrand) -> Option<&'static BrandRow> {
    BRANDS.get(brand.label())
}

fn version_major(version: &str) -> Option<u32> {
    version.split('.').next()?.parse().ok()
}

fn ua_seed(profile_user_agent: &str) -> u64 {
    profile_user_agent.bytes().fold(0u64, |acc, b| {
        acc.wrapping_mul(1099511628211).wrapping_add(u64::from(b))
    })
}

fn is_desktop(platform: Platform) -> bool {
    match platform {
        Platform::Windows | Platform::MacOS | Platform::Linux => true,
        Platform::Android | Platform::IOS => false,
        Platform::Host => is_desktop(Platform::detect_host()),
    }
}

const GREASE_CHARS: [char; 11] = [' ', '(', ':', '-', '.', '/', ')', ';', '=', '?', '_'];
const GREASE_VERSIONS: [&str; 3] = ["8", "99", "24"];
const BRAND_ORDERS: [[usize; 3]; 6] = [
    [0, 1, 2],
    [0, 2, 1],
    [1, 0, 2],
    [1, 2, 0],
    [2, 0, 1],
    [2, 1, 0],
];

fn cycle<T: Copy>(table: &[T], seed: u32) -> T {
    table[usize::try_from(seed).unwrap_or_default() % table.len()]
}

pub(crate) fn sec_ch_ua(chromium_major: u32, product: Option<(&str, u32)>) -> String {
    let grease = format!(
        "Not{}A{}Brand",
        cycle(&GREASE_CHARS, chromium_major),
        cycle(&GREASE_CHARS, chromium_major.wrapping_add(1)),
    );
    let entries = [
        Some((grease, cycle(&GREASE_VERSIONS, chromium_major).to_string())),
        Some(("Chromium".to_string(), chromium_major.to_string())),
        product.map(|(name, version)| (name.to_string(), version.to_string())),
    ];
    let mut slots: [Option<(String, String)>; 3] = Default::default();
    for (entry, slot) in entries
        .into_iter()
        .zip(cycle(&BRAND_ORDERS, chromium_major))
    {
        slots[slot] = entry;
    }
    slots
        .iter()
        .flatten()
        .map(|(name, version)| format!(r#""{name}";v="{version}""#))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests;
