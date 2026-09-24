use crate::profile::Platform;

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
pub enum BrandOverlayError {
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
    pub fn version_for(self, chromium_major: u32) -> Option<u32> {
        match self {
            Self::Chrome | Self::Edge => Some(chromium_major),
            Self::Opera => OPERA_PER_CHROMIUM
                .iter()
                .find_map(|(chromium, opera)| (*chromium == chromium_major).then_some(*opera)),
            Self::Vivaldi => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Chrome => "Chrome",
            Self::Edge => "Edge",
            Self::Opera => "Opera",
            Self::Vivaldi => "Vivaldi",
        }
    }

    pub fn overlay(
        self,
        chromium_major: u32,
        platform: Platform,
        profile_user_agent: &str,
    ) -> Result<Option<BrandOverlay>, BrandOverlayError> {
        match self {
            Self::Chrome => Ok(None),
            Self::Edge => edge_overlay(chromium_major, platform, profile_user_agent).map(Some),
            Self::Opera => opera_overlay(chromium_major, platform, profile_user_agent).map(Some),
            Self::Vivaldi => {
                vivaldi_overlay(chromium_major, platform, profile_user_agent).map(Some)
            }
        }
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BrandOverlay {
    pub user_agent: String,
    pub sec_ch_ua: String,
    pub extra_headers: Vec<(String, String)>,
    pub navigate_accept: Option<String>,
}

fn desktop_only(platform: Platform) -> bool {
    match platform {
        Platform::Windows | Platform::MacOS | Platform::Linux => true,
        Platform::Android | Platform::IOS => false,
        Platform::Host => desktop_only(Platform::detect_host()),
    }
}

fn edge_overlay(
    chromium_major: u32,
    platform: Platform,
    profile_user_agent: &str,
) -> Result<BrandOverlay, BrandOverlayError> {
    if !desktop_only(platform) {
        return Err(BrandOverlayError::Unverified {
            brand: ChromiumBrand::Edge,
            chromium_major,
            platform,
        });
    }
    Ok(BrandOverlay {
        user_agent: format!("{profile_user_agent} Edg/{chromium_major}.0.0.0"),
        sec_ch_ua: sec_ch_ua(chromium_major, Some(("Microsoft Edge", chromium_major))),
        extra_headers: Vec::new(),
        navigate_accept: None,
    })
}

const OPERA_PER_CHROMIUM: &[(u32, u32)] = &[
    (152, 136),
    (151, 135),
    (150, 134),
    (149, 133),
    (148, 132),
    (147, 131),
    (146, 130),
    (145, 129),
];

fn opera_overlay(
    chromium_major: u32,
    platform: Platform,
    profile_user_agent: &str,
) -> Result<BrandOverlay, BrandOverlayError> {
    if !desktop_only(platform) {
        return Err(BrandOverlayError::Unverified {
            brand: ChromiumBrand::Opera,
            chromium_major,
            platform,
        });
    }
    let opera_version = OPERA_PER_CHROMIUM
        .iter()
        .find_map(|(chromium, opera)| (*chromium == chromium_major).then_some(*opera))
        .ok_or(BrandOverlayError::Unverified {
            brand: ChromiumBrand::Opera,
            chromium_major,
            platform,
        })?;
    Ok(BrandOverlay {
        user_agent: format!("{profile_user_agent} OPR/{opera_version}.0.0.0"),
        sec_ch_ua: sec_ch_ua(chromium_major, Some(("Opera", opera_version))),
        extra_headers: Vec::new(),
        navigate_accept: None,
    })
}

const VIVALDI_BUILDS_PER_MAJOR: &[(u32, &[&str])] = &[(147, &["7.9.3970.59"])];

fn vivaldi_build_for(chromium_major: u32, fallback_seed: u64) -> Option<&'static str> {
    for (major, builds) in VIVALDI_BUILDS_PER_MAJOR {
        if *major == chromium_major {
            return Some(builds[(fallback_seed as usize) % builds.len()]);
        }
    }
    None
}

fn vivaldi_overlay(
    chromium_major: u32,
    platform: Platform,
    profile_user_agent: &str,
) -> Result<BrandOverlay, BrandOverlayError> {
    if !desktop_only(platform) {
        return Err(BrandOverlayError::Unverified {
            brand: ChromiumBrand::Vivaldi,
            chromium_major,
            platform,
        });
    }
    let seed = profile_user_agent.bytes().fold(0u64, |acc, b| {
        acc.wrapping_mul(1099511628211).wrapping_add(b as u64)
    });
    let build = vivaldi_build_for(chromium_major, seed).ok_or(BrandOverlayError::Unverified {
        brand: ChromiumBrand::Vivaldi,
        chromium_major,
        platform,
    })?;
    Ok(BrandOverlay {
        user_agent: format!("{profile_user_agent} Vivaldi/{build}"),
        sec_ch_ua: sec_ch_ua(chromium_major, None),
        extra_headers: Vec::new(),
        navigate_accept: None,
    })
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
