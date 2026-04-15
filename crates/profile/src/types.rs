//! Profile types — deserialized from TOML profile files.

use serde::Deserialize;
use std::collections::HashMap;

#[allow(missing_docs)]
/// A complete browser fingerprint profile, loaded from TOML.
#[derive(Debug, Clone, Deserialize)]
pub struct BrowserProfile {
    pub meta: ProfileMeta,
    pub tls: TlsProfile,
    pub h2: H2Profile,
    #[serde(default)]
    pub identity: HashMap<String, PlatformIdentity>,
}

#[allow(missing_docs)]
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
}

#[allow(missing_docs)]
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
    /// Fixed extension permutation indices (e.g. Firefox's deterministic order).
    #[serde(default)]
    pub extension_permutation: Option<Vec<u8>>,
    #[serde(default)]
    pub ech_grease: bool,
    /// Fixed ECH GREASE payload length in bytes (Chrome 131+ uses a fixed length).
    #[serde(default)]
    pub ech_grease_payload_len: Option<u16>,
    #[serde(default)]
    pub pre_shared_key: bool,
    #[serde(default)]
    pub fingerprint: Option<TlsFingerprint>,
}

#[allow(missing_docs)]
/// Expected TLS fingerprint for verification.
#[derive(Debug, Clone, Deserialize)]
pub struct TlsFingerprint {
    #[serde(default)]
    pub ja4: Option<String>,
}

#[allow(missing_docs)]
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
    #[serde(default)]
    pub fingerprint: Option<H2Fingerprint>,
}

#[allow(missing_docs)]
/// Expected H2 fingerprint for verification.
#[derive(Debug, Clone, Deserialize)]
pub struct H2Fingerprint {
    #[serde(default)]
    pub akamai: Option<String>,
}

#[allow(missing_docs)]
/// Platform-specific identity (user-agent, sec-ch-ua).
#[derive(Debug, Clone, Deserialize)]
pub struct PlatformIdentity {
    pub user_agent: String,
    pub sec_ch_ua: String,
    #[serde(default)]
    pub accept_language: Option<String>,
}

impl BrowserProfile {
    /// Parse a profile from a TOML string.
    pub fn from_toml(toml_str: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(toml_str)
    }

    /// Get the identity for a given platform.
    pub fn identity_for(&self, platform: &str) -> Option<&PlatformIdentity> {
        self.identity.get(platform)
    }

    /// Expected JA4 hash, if specified.
    pub fn expected_ja4(&self) -> Option<&str> {
        self.tls.fingerprint.as_ref()?.ja4.as_deref()
    }

    /// Expected Akamai H2 fingerprint, if specified.
    pub fn expected_h2_fingerprint(&self) -> Option<&str> {
        self.h2.fingerprint.as_ref()?.akamai.as_deref()
    }
}
