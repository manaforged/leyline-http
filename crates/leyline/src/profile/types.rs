//! Profile types — deserialized from TOML profile files.

use std::collections::HashMap;

use serde::Deserialize;

use crate::profile::registry::ProfileError;
use crate::{Error, Kind};

#[expect(
    missing_docs,
    reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
)]
/// A complete browser fingerprint profile, loaded from TOML.
#[derive(Debug, Clone, Deserialize)]
pub struct BrowserProfile {
    pub meta: ProfileMeta,
    pub tls: TlsProfile,
    pub h2: H2Profile,
    #[serde(default)]
    pub identity: HashMap<String, PlatformIdentity>,
}

#[expect(
    missing_docs,
    reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
)]
/// Profile metadata.
#[derive(Debug, Clone, Deserialize)]
pub struct ProfileMeta {
    pub name: String,
    pub browser: String,
    pub version: u32,
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub verified_against: String,
    /// The exact browser build this profile's fingerprint was captured against, e.g. `"chrome-150.0.7871.128"`.
    #[serde(default)]
    pub captured_against: Option<String>,
}

#[expect(
    missing_docs,
    reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
)]
/// TLS ClientHello configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct TlsProfile {
    pub ciphers: Vec<String>,
    pub curves: Vec<String>,
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
    /// Fixed ClientHello extension order as IANA TLS extension type IDs.
    #[serde(default)]
    pub extension_permutation: Option<Vec<u16>>,
    #[serde(default = "default_grease")]
    pub grease: bool,
    #[serde(default)]
    pub ech_grease: bool,
    #[serde(default)]
    pub pre_shared_key: bool,
    /// Advertise the TLS session_ticket extension (0x0023).
    #[serde(default = "default_true")]
    pub session_tickets: bool,
    /// Advertise the TLS Trust Anchor Identifiers extension (0xCA34/51764) with an empty list when the selected browser profile does.
    #[serde(default)]
    pub request_trust_anchors: bool,
    #[serde(default)]
    pub fingerprint: Option<TlsFingerprint>,
    /// BoringSSL appends RFC 7685 padding (0x0015) to this hello shape; it is never part of `extension_permutation` but counts toward the wire JA4.
    #[serde(default)]
    pub padding: bool,
    /// ClientHello supported_versions floor: "1.0" (CFNetwork iOS advertises TLS 1.0/1.1), "1.2" (default), "1.3".
    #[serde(default)]
    pub min_tls_version: Option<String>,
}

const fn default_grease() -> bool {
    true
}

const fn default_true() -> bool {
    true
}

#[expect(
    missing_docs,
    reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
)]
/// Expected TLS fingerprint for verification.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TlsFingerprint {
    #[serde(default)]
    pub ja4: Option<String>,
    /// JA4 observed on a resumed TLS 1.3 handshake that carries `pre_shared_key` (41).
    #[serde(default)]
    pub resumed_ja4: Option<String>,
    /// Per-platform JA4 overrides.
    #[serde(default)]
    pub platforms: HashMap<String, TlsFingerprint>,
}

#[expect(
    missing_docs,
    reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
)]
/// HTTP/2 SETTINGS frame configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct H2Profile {
    #[serde(default)]
    pub header_table_size: Option<u32>,
    #[serde(default)]
    pub enable_push: Option<bool>,
    #[serde(default)]
    pub max_concurrent_streams: Option<u32>,
    #[serde(default)]
    pub initial_stream_window_size: Option<u32>,
    #[serde(default)]
    pub initial_connection_window_size: Option<u32>,
    #[serde(default)]
    pub max_frame_size: Option<u32>,
    #[serde(default)]
    pub max_header_list_size: Option<u32>,
    #[serde(default)]
    pub unknown_setting8: Option<u32>,
    #[serde(default)]
    pub unknown_setting9: Option<u32>,
    pub pseudo_order: Vec<String>,
    pub settings_order: Vec<String>,
    /// PRIORITY fields emitted on each request's initial HEADERS frame.
    #[serde(default)]
    pub default_priority: Option<H2PriorityProfile>,
    #[serde(default)]
    pub fingerprint: Option<H2Fingerprint>,
    /// Per-platform overrides.
    #[serde(default)]
    pub platforms: HashMap<String, H2PlatformOverride>,
}

/// PRIORITY fields for the initial HEADERS frame (RFC 9113 §5.3, fingerprint parity for Chrome-class browsers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct H2PriorityProfile {
    /// Exclusive dependency bit (E).
    pub exclusive: bool,
    /// Stream ID the new stream depends on (0 = root).
    pub stream_dependency: u32,
    /// Wire weight byte (`actual_weight - 1`), range `0..=255`.
    pub weight: u8,
}

#[expect(
    missing_docs,
    reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
)]
/// Per-platform overrides for an [`H2Profile`].
#[derive(Debug, Clone, Default, Deserialize)]
pub struct H2PlatformOverride {
    #[serde(default)]
    pub header_table_size: Option<u32>,
    #[serde(default)]
    pub enable_push: Option<bool>,
    #[serde(default)]
    pub max_concurrent_streams: Option<u32>,
    #[serde(default)]
    pub initial_stream_window_size: Option<u32>,
    #[serde(default)]
    pub initial_connection_window_size: Option<u32>,
    #[serde(default)]
    pub max_frame_size: Option<u32>,
    #[serde(default)]
    pub max_header_list_size: Option<u32>,
    #[serde(default)]
    pub unknown_setting8: Option<u32>,
    #[serde(default)]
    pub unknown_setting9: Option<u32>,
    /// Names of base settings to omit from this platform's SETTINGS frame.
    #[serde(default)]
    pub omit_settings: Vec<String>,
    /// Optional pseudo_order override (rarely needed).
    #[serde(default)]
    pub pseudo_order: Option<Vec<String>>,
    /// Optional settings_order override.
    #[serde(default)]
    pub settings_order: Option<Vec<String>>,
    /// Per-platform Akamai fingerprint expectation.
    #[serde(default)]
    pub fingerprint: Option<H2Fingerprint>,
}

#[expect(
    missing_docs,
    reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
)]
/// Expected H2 fingerprint for verification.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct H2Fingerprint {
    #[serde(default)]
    pub akamai: Option<String>,
}

#[expect(
    missing_docs,
    reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
)]
/// Platform-specific identity (user-agent, sec-ch-ua).
#[derive(Debug, Clone, Deserialize)]
pub struct PlatformIdentity {
    pub user_agent: String,
    pub sec_ch_ua: String,
    #[serde(default)]
    pub accept_language: Option<String>,
    /// Optional explicit request-header order.
    #[serde(default)]
    pub request_header_order: Option<Vec<String>>,
    /// Extra headers appended for every request from this identity (for example Brave's `sec-gpc: 1`).
    #[serde(default)]
    pub extra_headers: Vec<(String, String)>,
    /// Optional override for the `accept` value emitted by the `Preset::Navigate` preset.
    #[serde(default)]
    pub navigate_accept_override: Option<String>,
}

impl BrowserProfile {
    /// Parse a profile from a TOML string; fails with [`ProfileError::Parse`] on invalid TOML or a rejected permutation.
    pub fn from_toml(toml_str: &str) -> Result<Self, ProfileError> {
        let profile: Self = toml::from_str(toml_str).map_err(ProfileError::parse)?;
        crate::profile::permutation::validate(&profile.tls)
            .map_err(|why| ProfileError::parse(format!("{}: {why}", profile.meta.name)))?;
        for warning in profile.load_warnings() {
            tracing::warn!(target: "leyline::profile", "{warning}");
        }
        Ok(profile)
    }

    /// Non-fatal load-time warnings for a parsed profile.
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

    /// Get the identity for a given platform.
    pub fn identity_for(&self, platform: crate::profile::Platform) -> Option<&PlatformIdentity> {
        self.identity.get(platform.identity_key())
    }

    /// Expected JA4 hash, if specified.
    pub fn expected_ja4(&self) -> Option<&str> {
        self.tls.fingerprint.as_ref()?.ja4.as_deref()
    }

    /// Expected JA4 for a resumed TLS 1.3 handshake, if captured.
    pub fn expected_resumed_ja4(&self) -> Option<&str> {
        self.tls.fingerprint.as_ref()?.resumed_ja4.as_deref()
    }

    /// Expected Akamai H2 fingerprint, if specified.
    pub fn expected_h2_fingerprint(&self) -> Option<&str> {
        self.h2.fingerprint.as_ref()?.akamai.as_deref()
    }

    /// Expected Akamai H2 fingerprint for a specific platform key.
    pub fn expected_h2_fingerprint_for(&self, platform: crate::profile::Platform) -> Option<&str> {
        if let Some(p) = self.h2.platforms.get(platform.identity_key())
            && let Some(ref fp) = p.fingerprint
            && let Some(ref s) = fp.akamai
        {
            return Some(s.as_str());
        }
        self.h2.fingerprint.as_ref()?.akamai.as_deref()
    }
}

impl H2Profile {
    /// Resolve this profile for a given platform.
    pub fn resolve_for_platform(
        &self,
        platform: crate::profile::Platform,
    ) -> Result<H2Profile, Error> {
        let Some(over) = self.platforms.get(platform.identity_key()) else {
            return Ok(self.clone());
        };
        let mut out = self.clone();

        if let Some(v) = over.header_table_size {
            out.header_table_size = Some(v);
        }
        if let Some(v) = over.enable_push {
            out.enable_push = Some(v);
        }
        if let Some(v) = over.max_concurrent_streams {
            out.max_concurrent_streams = Some(v);
        }
        if let Some(v) = over.initial_stream_window_size {
            out.initial_stream_window_size = Some(v);
        }
        if let Some(v) = over.initial_connection_window_size {
            out.initial_connection_window_size = Some(v);
        }
        if let Some(v) = over.max_frame_size {
            out.max_frame_size = Some(v);
        }
        if let Some(v) = over.max_header_list_size {
            out.max_header_list_size = Some(v);
        }
        if let Some(v) = over.unknown_setting8 {
            out.unknown_setting8 = Some(v);
        }
        if let Some(v) = over.unknown_setting9 {
            out.unknown_setting9 = Some(v);
        }
        if let Some(ref order) = over.pseudo_order {
            out.pseudo_order = order.clone();
        }
        if let Some(ref order) = over.settings_order {
            out.settings_order = order.clone();
        }

        for name in &over.omit_settings {
            match name.as_str() {
                "header_table_size" => out.header_table_size = None,
                "enable_push" => out.enable_push = None,
                "max_concurrent_streams" => out.max_concurrent_streams = None,
                "initial_stream_window_size" => out.initial_stream_window_size = None,
                "max_frame_size" => out.max_frame_size = None,
                "max_header_list_size" => out.max_header_list_size = None,
                "unknown_setting8" => out.unknown_setting8 = None,
                "unknown_setting9" => out.unknown_setting9 = None,
                other => {
                    return Err(Error::new(Kind::Config).with_message(format!(
                        "unknown name in [h2.platforms.{}].omit_settings: {other:?}",
                        platform.identity_key()
                    )));
                }
            }
        }

        Ok(out)
    }
}

#[cfg(test)]
mod tests;
