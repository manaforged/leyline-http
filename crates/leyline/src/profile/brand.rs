use std::sync::Arc;

use crate::error::Error;
use crate::profile::{BrowserHeaders, BrowserProfile, HeaderStyle, Platform, TlsProfile};

include!(concat!(env!("OUT_DIR"), "/chromium_brand.rs"));

impl std::fmt::Display for ChromiumBrand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id())
    }
}

string_id_serde!(ChromiumBrand);

impl std::str::FromStr for ChromiumBrand {
    type Err = Error;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        BRAND_ALL
            .iter()
            .zip(BRAND_KEYS)
            .find(|(_, key)| key.eq_ignore_ascii_case(name))
            .map(|(brand, _)| *brand)
            .ok_or_else(|| {
                crate::profile::browser::unknown("Chromium brand", name, BRAND_KEYS.iter().copied())
            })
    }
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
    #[must_use]
    pub fn id(&self) -> &'static str {
        BRAND_KEYS[*self as usize]
    }

    pub(crate) fn label(self) -> &'static str {
        BRAND_LABELS[self as usize]
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
        let Some(token) = row.ua_token else {
            return Ok(None);
        };
        if row.desktop_only && !is_desktop(platform) {
            return Err(unverified());
        }
        let version = row
            .version(chromium_major, ua_seed(profile_user_agent))
            .ok_or_else(unverified)?;
        let product = match row.product {
            Some(name) => Some((name, version_major(&version).ok_or_else(unverified)?)),
            None => None,
        };
        Ok(Some(BrandOverlay {
            user_agent: format!("{profile_user_agent} {token}/{version}"),
            sec_ch_ua: sec_ch_ua(chromium_major, product),
        }))
    }
}

impl ChromiumBrand {
    pub(crate) fn apply(
        self,
        chromium_major: u32,
        platform: Platform,
        identity: &mut BrowserHeaders,
    ) -> Result<(), BrandOverlayError> {
        if let Some(overlay) = self.overlay(chromium_major, platform, &identity.user_agent)? {
            identity.user_agent = overlay.user_agent;
            identity.sec_ch_ua = overlay.sec_ch_ua;
        }
        Ok(())
    }

    pub(crate) fn header_style(self) -> Option<HeaderStyle> {
        brand_row(self).and_then(|row| row.header_style)
    }

    pub(crate) fn tls_profile(self, profile: Arc<BrowserProfile>) -> Arc<BrowserProfile> {
        let quic_tls = profile.h3.as_ref().and_then(|h3| h3.tls.as_ref());
        let Some(tls) = brand_row(self).map(|row| &row.tls).filter(|tls| {
            tls.changes(&profile.tls) || quic_tls.is_some_and(|quic| tls.changes(quic))
        }) else {
            return profile;
        };
        let mut branded = (*profile).clone();
        tls.apply(&mut branded.tls);
        if let Some(quic) = branded.h3.as_mut().and_then(|h3| h3.tls.as_mut()) {
            tls.apply(quic);
        }
        Arc::new(branded)
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub(crate) struct BrandOverlay {
    pub(crate) user_agent: String,
    pub(crate) sec_ch_ua: String,
}

#[derive(Debug)]
struct BrandRow {
    product: Option<&'static str>,
    ua_token: Option<&'static str>,
    follow_chromium: bool,
    desktop_only: bool,
    versions: &'static [(&'static str, &'static [&'static str])],
    header_style: Option<HeaderStyle>,
    tls: BrandTls,
}

#[derive(Debug)]
struct BrandTls {
    request_trust_anchors: Option<bool>,
}

impl BrandTls {
    fn changes(&self, tls: &TlsProfile) -> bool {
        self.request_trust_anchors
            .is_some_and(|request| request != tls.request_trust_anchors)
    }

    fn apply(&self, tls: &mut TlsProfile) {
        if let Some(request) = self.request_trust_anchors {
            tls.request_trust_anchors = request;
        }
        tls.fingerprint = None;
    }
}

impl BrandRow {
    fn version(&self, chromium_major: u32, seed: u64) -> Option<String> {
        if self.follow_chromium {
            return Some(format!("{chromium_major}.0.0.0"));
        }
        let major = chromium_major.to_string();
        let builds = self.builds(&major)?;
        let count = u64::try_from(builds.len()).ok().filter(|n| *n > 0)?;
        builds
            .get(usize::try_from(seed % count).ok()?)
            .map(|build| (*build).to_owned())
    }

    fn builds(&self, chromium_major: &str) -> Option<&'static [&'static str]> {
        self.versions
            .iter()
            .find(|(major, _)| *major == chromium_major)
            .map(|(_, builds)| *builds)
    }
}

fn brand_row(brand: ChromiumBrand) -> Option<&'static BrandRow> {
    BRAND_ROWS.get(brand as usize)
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
