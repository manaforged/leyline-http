use std::sync::LazyLock;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use leyline_bssl::ed25519::{self, PUBLIC_KEY_LEN, SIGNATURE_LEN};
use serde::Deserialize;

use crate::core::Session;
use crate::profile::registry::{ProfileError, ProfileRegistry};
use crate::profile::types::BrowserProfile;
use crate::{Error, Kind};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceData {
    url: String,
    public_keys: Vec<String>,
    max_bytes: usize,
    format: u32,
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
    generated: String,
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
}

impl BundleSource {
    #[must_use]
    pub fn new(url: impl Into<String>, public_keys: Vec<[u8; PUBLIC_KEY_LEN]>) -> Self {
        Self {
            url: url.into(),
            public_keys,
            max_bytes: OFFICIAL.max_bytes,
        }
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
        Self::new(&OFFICIAL.url, keys)
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
    generated: String,
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
        if wire.format != OFFICIAL.format {
            return Err(ProfileError::Format {
                format: wire.format,
            });
        }
        let mut profiles = ProfileRegistry::new();
        let mut skipped = Vec::new();
        for entry in wire.profiles {
            match BrowserProfile::from_toml(&entry.toml) {
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
            generated: wire.generated,
            skipped,
        })
    }

    #[must_use]
    pub fn profiles(&self) -> &ProfileRegistry {
        &self.profiles
    }

    #[must_use]
    pub fn generated(&self) -> &str {
        &self.generated
    }

    #[must_use]
    pub fn skipped(&self) -> &[SkippedProfile] {
        &self.skipped
    }
}

async fn download(session: &Session, url: &str, max_bytes: usize) -> crate::Result<Bytes> {
    let response = session.get(url).send().await?.error_for_status()?;
    let too_large =
        || Error::new(Kind::Body).with_message(format!("{url} is larger than {max_bytes} bytes"));
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(too_large());
    }
    let body = response.bytes().await?;
    if body.len() > max_bytes {
        return Err(too_large());
    }
    Ok(body)
}

#[cfg(test)]
mod tests;
