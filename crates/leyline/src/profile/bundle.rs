use std::sync::LazyLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use leyline_bssl::ed25519::{self, PUBLIC_KEY_LEN, SIGNATURE_LEN};
use serde::Deserialize;

use crate::core::Session;
use crate::profile::registry::{ProfileError, ProfileRegistry};
use crate::profile::types::BrowserProfile;
use crate::{Error, Kind};

mod policy;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceData {
    url: String,
    public_keys: Vec<String>,
    max_bytes: usize,
    format: u32,
    min_serial: u64,
    clock_skew_secs: u64,
    max_lifetime_secs: u64,
    policy: PolicyData,
    #[serde(rename = "signer")]
    _signer: serde::de::IgnoredAny,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyData {
    min_tls_version: String,
}

static OFFICIAL: LazyLock<SourceData> = LazyLock::new(|| {
    toml::from_str(include_str!("../../profiles/bundle.toml"))
        .expect("built-in bundle source is statically valid")
});

#[derive(Deserialize)]
struct Envelope {
    payload: String,
    signature: String,
}

#[derive(Deserialize)]
struct Wire {
    format: u32,
    serial: u64,
    expires: u64,
    profiles: Vec<WireProfile>,
}

#[derive(Deserialize)]
struct WireProfile {
    browser: String,
    version: u32,
    toml: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleSource {
    url: String,
    public_keys: Vec<[u8; PUBLIC_KEY_LEN]>,
    max_bytes: usize,
    min_serial: u64,
    max_lifetime: Duration,
}

impl BundleSource {
    #[must_use]
    pub fn new(url: impl Into<String>, public_keys: Vec<[u8; PUBLIC_KEY_LEN]>) -> Self {
        Self {
            url: url.into(),
            public_keys,
            max_bytes: OFFICIAL.max_bytes,
            min_serial: 0,
            max_lifetime: Duration::from_secs(OFFICIAL.max_lifetime_secs),
        }
    }

    #[must_use]
    pub fn max_lifetime(mut self, lifetime: Duration) -> Self {
        self.max_lifetime = lifetime;
        self
    }

    #[must_use]
    pub fn min_serial(mut self, serial: u64) -> Self {
        self.min_serial = self.min_serial.max(serial);
        self
    }

    #[must_use]
    pub fn max_bytes(mut self, bytes: usize) -> Self {
        self.max_bytes = bytes;
        self
    }

    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }
}

impl Default for BundleSource {
    fn default() -> Self {
        let keys = OFFICIAL
            .public_keys
            .iter()
            .map(|key| {
                let mut raw = [0; PUBLIC_KEY_LEN];
                hex::decode_to_slice(key, &mut raw).expect("built-in bundle key is 32 hex bytes");
                raw
            })
            .collect();
        Self::new(&OFFICIAL.url, keys).min_serial(OFFICIAL.min_serial)
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub struct SkippedProfile {
    pub browser: String,
    pub version: u32,
    pub error: ProfileError,
}

#[derive(Debug)]
pub struct ProfileBundle {
    profiles: ProfileRegistry,
    serial: u64,
    expires: SystemTime,
    skipped: Vec<SkippedProfile>,
}

impl ProfileBundle {
    pub async fn fetch(session: &Session, source: &BundleSource) -> crate::Result<Self> {
        let bundle = download(session, &source.url, source.max_bytes).await?;
        Self::from_bytes(&bundle, source)
            .map_err(|error| Error::new(Kind::Decode).with_source(error))
    }

    pub fn from_bytes(bundle: &[u8], source: &BundleSource) -> Result<Self, ProfileError> {
        let envelope: Envelope = serde_json::from_slice(bundle).map_err(ProfileError::parse)?;
        let payload = STANDARD
            .decode(&envelope.payload)
            .map_err(ProfileError::parse)?;
        let mut signature = [0; SIGNATURE_LEN];
        if hex::decode_to_slice(&envelope.signature, &mut signature).is_err() {
            return Err(ProfileError::Signature);
        }
        let signed = source
            .public_keys
            .iter()
            .any(|key| ed25519::verify(&payload, &signature, key));
        if !signed {
            return Err(ProfileError::Signature);
        }
        let wire: Wire = serde_json::from_slice(&payload).map_err(ProfileError::parse)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let expires = check_current(&wire, source, now)?;
        let mut profiles = ProfileRegistry::new();
        let mut skipped = Vec::new();
        for entry in wire.profiles {
            let loaded = BrowserProfile::from_toml(&entry.toml).and_then(|profile| {
                policy::check(&entry.browser, entry.version, &profile).map(|()| profile)
            });
            match loaded {
                Ok(profile) => profiles.insert(profile),
                Err(error) => skipped.push(SkippedProfile {
                    browser: entry.browser,
                    version: entry.version,
                    error,
                }),
            }
        }
        Ok(Self {
            profiles,
            serial: wire.serial,
            expires,
            skipped,
        })
    }

    #[must_use]
    pub fn profiles(&self) -> &ProfileRegistry {
        &self.profiles
    }

    #[must_use]
    pub fn serial(&self) -> u64 {
        self.serial
    }

    #[must_use]
    pub fn expires(&self) -> SystemTime {
        self.expires
    }

    #[must_use]
    pub fn skipped(&self) -> &[SkippedProfile] {
        &self.skipped
    }
}

fn check_current(wire: &Wire, source: &BundleSource, now: u64) -> Result<SystemTime, ProfileError> {
    if wire.format != OFFICIAL.format {
        return Err(ProfileError::Format {
            format: wire.format,
        });
    }
    if wire.serial < source.min_serial {
        return Err(ProfileError::Rollback {
            serial: wire.serial,
            min_serial: source.min_serial,
        });
    }
    if wire.serial > now.saturating_add(OFFICIAL.clock_skew_secs) {
        return Err(ProfileError::FutureSerial {
            serial: wire.serial,
        });
    }
    if now >= wire.expires {
        return Err(ProfileError::Expired {
            expires: wire.expires,
        });
    }
    let lifetime = Duration::from_secs(wire.expires.saturating_sub(wire.serial));
    match UNIX_EPOCH.checked_add(Duration::from_secs(wire.expires)) {
        Some(expires) if lifetime <= source.max_lifetime => Ok(expires),
        _ => Err(ProfileError::Lifetime {
            serial: wire.serial,
            expires: wire.expires,
        }),
    }
}

async fn download(session: &Session, url: &str, max_bytes: usize) -> crate::Result<Vec<u8>> {
    let response = session.get(url).stream().send().await?.error_for_status()?;
    let mut body = Vec::new();
    response
        .copy_decoded_to(&mut body, Some(max_bytes as u64))
        .await?;
    Ok(body)
}

#[cfg(test)]
mod policy_tests;
#[cfg(test)]
mod tests;
