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
        profile_sec_ch_ua: &str,
    ) -> Result<Option<BrandOverlay>, BrandOverlayError> {
        match self {
            Self::Chrome => Ok(None),
            Self::Edge => edge_overlay(
                chromium_major,
                platform,
                profile_user_agent,
                profile_sec_ch_ua,
            )
            .map(Some),
            Self::Opera => opera_overlay(chromium_major, platform, profile_user_agent).map(Some),
            Self::Vivaldi => vivaldi_overlay(
                chromium_major,
                platform,
                profile_user_agent,
                profile_sec_ch_ua,
            )
            .map(Some),
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
    profile_sec_ch_ua: &str,
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
        sec_ch_ua: swap_brand(profile_sec_ch_ua, "Microsoft Edge"),
        extra_headers: vec![("dnt".into(), "1".into())],
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
        sec_ch_ua: format!(
            r#""Not:A-Brand";v="99", "Opera";v="{opera_version}", "Chromium";v="{chromium_major}""#
        ),
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
    profile_sec_ch_ua: &str,
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
        sec_ch_ua: drop_brand(profile_sec_ch_ua, "Google Chrome"),
        extra_headers: Vec::new(),
        navigate_accept: None,
    })
}

pub(crate) fn swap_brand(chrome_sec_ch_ua: &str, new_brand: &str) -> String {
    const CHROME_NAME: &str = r#""Google Chrome""#;
    let mut swapped = false;
    let result = chrome_sec_ch_ua
        .split(',')
        .map(|entry| {
            let ws_len = entry
                .bytes()
                .take_while(|b| b.is_ascii_whitespace())
                .count();
            let (lead, rest) = entry.split_at(ws_len);
            if let Some(tail) = rest.strip_prefix(CHROME_NAME) {
                swapped = true;
                format!("{lead}\"{new_brand}\"{tail}")
            } else {
                entry.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    debug_assert!(
        swapped,
        "swap_brand called with sec-ch-ua that does not contain `\"Google Chrome\"`: \
         {chrome_sec_ch_ua:?}",
    );
    result
}

pub(crate) fn drop_brand(chrome_sec_ch_ua: &str, brand_name: &str) -> String {
    let needle = format!("\"{brand_name}\"");
    let mut dropped = false;
    let result = chrome_sec_ch_ua
        .split(',')
        .filter(|entry| {
            let trimmed = entry.trim_start();
            if trimmed.starts_with(&needle) {
                dropped = true;
                false
            } else {
                true
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    debug_assert!(
        dropped,
        "drop_brand called with sec-ch-ua that does not contain {needle}: {chrome_sec_ch_ua:?}",
    );
    result.trim_start().to_string()
}

#[cfg(test)]
mod tests;
