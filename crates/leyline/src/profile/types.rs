use std::collections::HashMap;

use serde::Deserialize;

use crate::error::Error;
use crate::profile::HeaderStyle;
use crate::profile::{Carrier, H2Settings, H3Profile, SettingId};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct BrowserProfile {
    pub meta: ProfileMeta,
    pub tls: TlsProfile,
    pub h2: H2Profile,
    #[serde(default)]
    pub identity: HashMap<String, BrowserHeaders>,
    #[serde(default)]
    pub h3: Option<H3Profile>,
    #[serde(skip)]
    source: Option<source::ProfileSource>,
    #[serde(default)]
    fingerprint: Vec<crate::profile::fingerprint::FingerprintLayer>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ProfileMeta {
    pub name: String,
    pub browser: String,
    pub version: u32,
    #[cfg_attr(
        not(feature = "bench-internals"),
        expect(
            dead_code,
            reason = "the TOML schema keeps this key; only bench-internals reads it"
        )
    )]
    #[serde(default)]
    pub family: String,
    #[cfg_attr(
        not(feature = "bench-internals"),
        expect(
            dead_code,
            reason = "the TOML schema keeps this key; only bench-internals reads it"
        )
    )]
    #[serde(default)]
    pub verified_against: String,
    #[serde(default)]
    pub captured_against: Option<String>,
    #[serde(default)]
    pub chromium_major: Option<u32>,
    #[serde(default)]
    pub ch_ua_brand: Option<String>,
    #[serde(default)]
    pub header_style: HeaderStyle,
    #[serde(skip)]
    pub(crate) header_order: Option<Vec<String>>,
    #[serde(default, rename = "capture")]
    _capture: serde::de::IgnoredAny,
    #[serde(default, rename = "variant")]
    _variant: serde::de::IgnoredAny,
    #[serde(default, rename = "hello")]
    _hello: serde::de::IgnoredAny,
    #[serde(default, rename = "platform_browser")]
    _platform_browser: serde::de::IgnoredAny,
    #[serde(default, rename = "deprecated")]
    _deprecated: serde::de::IgnoredAny,
    #[serde(default, rename = "verified_at")]
    _verified_at: serde::de::IgnoredAny,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TlsProfile {
    pub ciphers: Vec<String>,
    pub curves: Vec<String>,
    #[serde(default)]
    pub key_shares: Option<Vec<String>>,
    pub sigalgs: Vec<String>,
    #[serde(default)]
    pub delegated_credentials: Option<String>,
    #[serde(default)]
    pub record_size_limit: Option<u16>,
    #[serde(default)]
    pub alps: Option<String>,
    #[serde(default)]
    pub alps_new_codepoint: bool,
    #[serde(default)]
    pub ocsp_stapling: bool,
    #[serde(default)]
    pub signed_cert_timestamps: bool,
    #[serde(default)]
    pub cert_compression: Vec<String>,
    #[serde(default)]
    pub permute_extensions: bool,
    #[serde(default)]
    pub extension_permutation: Option<Vec<u16>>,
    #[serde(default)]
    pub extension_tail: Vec<u16>,
    #[serde(default = "default_true")]
    pub grease: bool,
    #[serde(default)]
    pub sigalg_grease: bool,
    #[serde(default)]
    pub ech_grease: bool,
    #[serde(default)]
    pub pre_shared_key: bool,
    #[serde(default = "default_true")]
    pub session_tickets: bool,
    #[serde(default)]
    pub request_trust_anchors: bool,
    #[serde(default)]
    pub fingerprint: Option<TlsFingerprint>,
    #[serde(default)]
    pub padding: bool,
    #[serde(default)]
    pub min_tls_version: Option<String>,
    #[serde(default)]
    pub tls12_extensions: bool,
}

const fn default_true() -> bool {
    true
}

/// A certificate compression algorithm of RFC 8879 that a profile may list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CertCompression {
    Brotli,
    Zlib,
    Zstd,
}

impl CertCompression {
    pub(crate) fn parse(id: &str) -> Result<Self, String> {
        match id {
            "brotli" => Ok(Self::Brotli),
            "zlib" => Ok(Self::Zlib),
            "zstd" => Ok(Self::Zstd),
            other => Err(format!("unknown cert compression algorithm: {other:?}")),
        }
    }
}

/// The lowest TLS version a handshake accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum TlsMinVersion {
    /// TLS 1.0.
    Tls10,
    /// TLS 1.2.
    Tls12,
    /// TLS 1.3.
    Tls13,
}

impl TlsMinVersion {
    pub(crate) fn parse(id: &str) -> Result<Self, String> {
        match id {
            "1.0" => Ok(Self::Tls10),
            "1.2" => Ok(Self::Tls12),
            "1.3" => Ok(Self::Tls13),
            other => Err(format!(
                "unsupported min_tls_version {other:?}; expected \"1.0\", \"1.2\", or \"1.3\""
            )),
        }
    }
}

impl TlsProfile {
    pub(super) fn validate_ids(&self) -> Result<(), String> {
        for algo in self.cert_compressions() {
            algo?;
        }
        self.min_version()?;
        self.key_share_ids()?;
        Ok(())
    }

    /// Parses `cert_compression` in profile order.
    pub(crate) fn cert_compressions(
        &self,
    ) -> impl Iterator<Item = Result<CertCompression, String>> + '_ {
        self.cert_compression
            .iter()
            .map(|id| CertCompression::parse(id))
    }

    /// Parses `min_tls_version`, which is `None` when the profile declares no minimum.
    pub(crate) fn min_version(&self) -> Result<Option<TlsMinVersion>, String> {
        self.min_tls_version
            .as_deref()
            .map(TlsMinVersion::parse)
            .transpose()
    }

    /// Resolves `key_shares` to group ids, which must be an ordered subsequence of `curves`.
    pub(crate) fn key_share_ids(&self) -> Result<Option<Vec<u16>>, String> {
        let Some(names) = self.key_shares.as_deref() else {
            return Ok(None);
        };
        let ids = names
            .iter()
            .map(|name| {
                crate::iana::curve_id(name)
                    .ok_or_else(|| format!("unknown key share group: {name}"))
            })
            .collect::<Result<Vec<u16>, _>>()?;
        let mut curves = self
            .curves
            .iter()
            .filter_map(|name| crate::iana::curve_id(name));
        if !ids.iter().all(|id| curves.any(|curve| curve == *id)) {
            return Err(format!(
                "key_shares {names:?} must be an ordered subsequence of curves {:?}",
                self.curves
            ));
        }
        Ok(Some(ids))
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TlsFingerprint {
    #[cfg_attr(
        not(any(test, feature = "bench-internals")),
        expect(
            dead_code,
            reason = "the TOML schema keeps this key; only tests and bench-internals read it"
        )
    )]
    #[serde(default)]
    pub ja4: Option<String>,
    #[cfg_attr(
        not(feature = "bench-internals"),
        expect(
            dead_code,
            reason = "the documented TOML schema accepts this key; the library does not read it"
        )
    )]
    #[serde(default)]
    pub resumed_ja4: Option<String>,
    #[cfg_attr(
        not(feature = "bench-internals"),
        expect(
            dead_code,
            reason = "the TOML schema keeps this key; only bench-internals reads it"
        )
    )]
    #[serde(default)]
    pub platforms: HashMap<String, TlsFingerprint>,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct H2Profile {
    pub settings: H2Settings,
    pub initial_connection_window_size: Option<u32>,
    pub pseudo_order: Vec<String>,
    pub settings_order: Vec<String>,
    pub default_priority: Option<H2PriorityProfile>,
    pub fingerprint: Option<H2Fingerprint>,
    pub platforms: HashMap<String, H2PlatformOverride>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct H2PriorityProfile {
    pub exclusive: bool,
    pub stream_dependency: u32,
    pub weight: u8,
}

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct H2PlatformOverride {
    pub settings: H2Settings,
    pub initial_connection_window_size: Option<u32>,
    pub omit_settings: Vec<String>,
    pub pseudo_order: Option<Vec<String>>,
    pub settings_order: Option<Vec<String>>,
    #[cfg_attr(
        not(feature = "bench-internals"),
        expect(
            dead_code,
            reason = "the TOML schema keeps this key; only bench-internals reads it"
        )
    )]
    pub fingerprint: Option<H2Fingerprint>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct H2Fingerprint {
    #[cfg_attr(
        not(any(test, feature = "bench-internals")),
        expect(
            dead_code,
            reason = "the TOML schema keeps this key; only tests and bench-internals read it"
        )
    )]
    #[serde(default)]
    pub akamai: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct BrowserHeaders {
    pub user_agent: String,
    #[serde(default)]
    pub sec_ch_ua: String,
    #[serde(default)]
    pub accept_language: Option<String>,
}

fn invalid_profile(why: impl std::fmt::Display) -> crate::error::Error {
    Error::config(format!("invalid profile: {why}"))
}

impl BrowserProfile {
    pub fn from_toml(toml_str: &str) -> crate::error::Result<Self> {
        let mut profile: Self = toml::from_str(toml_str).map_err(invalid_profile)?;
        profile.derive_sec_ch_ua();
        profile.source = Some(source::ProfileSource::new(toml_str));
        profile
            .tls
            .validate(Carrier::Tcp)
            .map_err(|why| invalid_profile(format!("{}: {why}", profile.meta.name)))?;
        if let Some(h3) = &profile.h3 {
            h3.validate()
                .map_err(|why| invalid_profile(format!("{}: {why}", profile.meta.name)))?;
        }
        for layer in std::mem::take(&mut profile.fingerprint) {
            layer.apply(&mut profile)?;
        }
        for warning in profile.load_warnings() {
            tracing::warn!(target: "leyline::profile", "{warning}");
        }
        Ok(profile)
    }

    fn derive_sec_ch_ua(&mut self) {
        let Some(brand) = self.meta.ch_ua_brand.as_deref() else {
            return;
        };
        let derived =
            crate::profile::brand::sec_ch_ua(self.meta.version, Some((brand, self.meta.version)));
        for identity in self.identity.values_mut() {
            if identity.sec_ch_ua.is_empty() {
                identity.sec_ch_ua.clone_from(&derived);
            }
        }
    }

    pub fn load_warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        let capture_unrecorded = self
            .meta
            .captured_against
            .as_deref()
            .is_none_or(|s| s.trim().is_empty());
        if capture_unrecorded {
            warnings.push(format!(
                "{}: [meta] captured_against is missing — the exact browser build this \
                 profile was captured against is unrecorded",
                self.meta.name
            ));
        }
        warnings
    }

    pub fn identity_for(&self, platform: crate::profile::Platform) -> Option<&BrowserHeaders> {
        self.identity.get(platform.identity_key())
    }

    pub(crate) fn source(&self) -> Option<&str> {
        self.source.as_ref().map(source::ProfileSource::as_str)
    }
}

impl H2Profile {
    pub fn resolve_for_platform(
        &self,
        platform: crate::profile::Platform,
    ) -> Result<H2Profile, Error> {
        let Some(over) = self.platforms.get(platform.identity_key()) else {
            return Ok(self.clone());
        };
        let mut out = self.clone();
        let config = |why: String| Error::config(why);

        for id in SettingId::all() {
            if let Some(v) = over.settings.get(id) {
                out.settings.set(id, Some(v)).map_err(config)?;
            }
        }
        if let Some(v) = over.initial_connection_window_size {
            out.initial_connection_window_size = Some(v);
        }
        if let Some(ref order) = over.pseudo_order {
            out.pseudo_order = order.clone();
        }
        if let Some(ref order) = over.settings_order {
            out.settings_order = order.clone();
        }

        for name in &over.omit_settings {
            let Some(id) = SettingId::from_field(name) else {
                return Err(config(format!(
                    "unknown name in [h2.platforms.{}].omit_settings: {name:?}",
                    platform.identity_key()
                )));
            };
            out.settings.set(id, None).map_err(config)?;
        }

        Ok(out)
    }
}

mod h2_serde;
mod source;

#[cfg(test)]
mod tests;
