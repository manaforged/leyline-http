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
    /// Fixed ClientHello extension order as IANA TLS extension type IDs.
    #[serde(default)]
    pub extension_permutation: Option<Vec<u16>>,
    #[serde(default = "default_grease")]
    pub grease: bool,
    #[serde(default)]
    pub ech_grease: bool,
    #[serde(default)]
    pub pre_shared_key: bool,
    /// Advertise the TLS Trust Anchor Identifiers extension (0xCA34/51764) with
    /// an empty list when the selected browser profile does. This changes the
    /// ClientHello JA4 extension count, so each Chrome major must follow its
    /// captured wire profile.
    #[serde(default)]
    pub request_trust_anchors: bool,
    #[serde(default)]
    pub fingerprint: Option<TlsFingerprint>,
}

const fn default_grease() -> bool {
    true
}

#[allow(missing_docs)]
/// Expected TLS fingerprint for verification.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TlsFingerprint {
    #[serde(default)]
    pub ja4: Option<String>,
    /// JA4 observed on a resumed TLS 1.3 handshake that carries
    /// `pre_shared_key` (41). Cold first-flight JA4 remains in [`Self::ja4`].
    #[serde(default)]
    pub resumed_ja4: Option<String>,
    /// Per-platform JA4 overrides. Resolves Windows / macOS variation when
    /// the same browser ships different ClientHello configurations per
    /// host OS.
    #[serde(default)]
    pub platforms: HashMap<String, TlsFingerprint>,
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
    /// Per-platform overrides. The resolver in
    /// [`H2Profile::resolve_for_platform`] applies the override on top of
    /// this base profile to handle cases like Chromium-on-macOS dropping
    /// `unknown_setting8`.
    #[serde(default)]
    pub platforms: HashMap<String, H2PlatformOverride>,
}

#[allow(missing_docs)]
/// Per-platform overrides for an [`H2Profile`].
///
/// Each `Some` field overrides the base profile's matching field. Names
/// listed in `omit_settings` cause the corresponding base field to be set
/// to `None` (so that SETTINGS parameter is dropped from the wire frame).
/// `fingerprint` overrides the expected Akamai string for this platform.
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
    /// Valid names: `unknown_setting8`, `unknown_setting9`, plus any of the
    /// regular setting names (`header_table_size`, etc.).
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

#[allow(missing_docs)]
/// Expected H2 fingerprint for verification.
#[derive(Debug, Clone, Default, Deserialize)]
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
    /// Optional explicit request-header order. When set, the assembled
    /// request headers (after preset, brand overlay, identity extras,
    /// caller extras, content-length, cookie, referer, priority) are
    /// reordered to match this list. Names not in the list keep their
    /// relative order at the end. Used for browsers like Brave that ship
    /// a non-Chrome header sequence (e.g. `accept-language` repositioned
    /// between `accept` and `sec-fetch-*`).
    #[serde(default)]
    pub request_header_order: Option<Vec<String>>,
    /// Extra headers appended for every request from this identity (for
    /// example Brave's `sec-gpc: 1`). Caller-supplied and preset-supplied
    /// headers of the same name take precedence.
    #[serde(default)]
    pub extra_headers: Vec<(String, String)>,
    /// Optional override for the `accept` value emitted by the
    /// `Preset::Navigate` preset. Used by Brave to drop the
    /// `application/signed-exchange;v=b3;q=0.7` token.
    #[serde(default)]
    pub navigate_accept_override: Option<String>,
}

impl BrowserProfile {
    /// Parse a profile from a TOML string.
    pub fn from_toml(toml_str: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(toml_str)
    }

    /// The synthetic **bare** profile: a plain, non-impersonating HTTP
    /// client. This is what a [`Session`](crate::Session) is by default when
    /// no `.browser(...)` is chosen — a generic `leyline/<version>`
    /// User-Agent, no `sec-ch-ua`/brand headers, a minimal modern TLS
    /// cipher/curve/sigalg set with none of the browser-specific quirks
    /// (no ALPS, no cert-compression advert, no extension permutation,
    /// no delegated credentials, no record-size-limit), and stock HTTP/2
    /// SETTINGS. It carries no expected fingerprint — there is nothing to
    /// verify against an external browser.
    ///
    /// Built in code rather than from TOML so the default path needs no
    /// profile file and no parse step.
    pub fn bare() -> Self {
        // Generic, OS-agnostic identity. The UA is the same on every
        // platform; only the TCP fingerprint follows the resolved host.
        let identity = PlatformIdentity {
            user_agent: concat!("leyline/", env!("CARGO_PKG_VERSION")).to_string(),
            sec_ch_ua: String::new(),
            accept_language: Some("en-US,en;q=0.9".to_string()),
            request_header_order: None,
            extra_headers: Vec::new(),
            navigate_accept_override: None,
        };
        let mut identity_map = HashMap::new();
        for key in ["windows", "macos", "linux", "android", "ios"] {
            identity_map.insert(key.to_string(), identity.clone());
        }

        BrowserProfile {
            meta: ProfileMeta {
                name: "Bare (no impersonation)".to_string(),
                browser: "bare".to_string(),
                version: 0,
                family: "bare".to_string(),
                // Not anchored to any browser — exempt from verification.
                verified_against: "n/a (synthetic non-impersonating profile)".to_string(),
            },
            tls: TlsProfile {
                // Minimal modern set; names match the IANA-style spelling the
                // other profiles use (accepted by BoringSSL's cipher parser).
                ciphers: vec![
                    "TLS_AES_128_GCM_SHA256".into(),
                    "TLS_AES_256_GCM_SHA384".into(),
                    "TLS_CHACHA20_POLY1305_SHA256".into(),
                    "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256".into(),
                    "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256".into(),
                    "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384".into(),
                    "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384".into(),
                    "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256".into(),
                    "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256".into(),
                ],
                curves: vec!["X25519".into(), "SECP256R1".into(), "SECP384R1".into()],
                sigalgs: vec![
                    "ecdsa_secp256r1_sha256".into(),
                    "rsa_pss_rsae_sha256".into(),
                    "rsa_pkcs1_sha256".into(),
                    "ecdsa_secp384r1_sha384".into(),
                    "rsa_pss_rsae_sha384".into(),
                    "rsa_pkcs1_sha384".into(),
                ],
                delegated_credentials: None,
                record_size_limit: None,
                alps: None,
                alps_new_codepoint: false,
                ocsp_stapling: false,
                signed_cert_timestamps: false,
                cert_compression: Vec::new(),
                permute_extensions: false,
                extension_permutation: None,
                grease: false,
                ech_grease: false,
                pre_shared_key: false,
                request_trust_anchors: false,
                fingerprint: None,
            },
            h2: H2Profile {
                header_table_size: Some(4096),
                enable_push: Some(false),
                max_concurrent_streams: Some(100),
                initial_stream_window_size: Some(65535),
                initial_connection_window_size: Some(65535),
                max_frame_size: Some(16384),
                max_header_list_size: None,
                unknown_setting8: None,
                unknown_setting9: None,
                pseudo_order: vec![
                    "method".into(),
                    "scheme".into(),
                    "authority".into(),
                    "path".into(),
                ],
                settings_order: vec![
                    "header_table_size".into(),
                    "enable_push".into(),
                    "max_concurrent_streams".into(),
                    "initial_window_size".into(),
                    "max_frame_size".into(),
                ],
                fingerprint: None,
                platforms: HashMap::new(),
            },
            identity: identity_map,
        }
    }

    /// Get the identity for a given platform.
    pub fn identity_for(&self, platform: crate::profile::Platform) -> Option<&PlatformIdentity> {
        self.identity.get(platform.identity_key())
    }

    /// Expected JA4 hash, if specified. Returns the base profile's JA4
    /// (typically the Windows form).
    pub fn expected_ja4(&self) -> Option<&str> {
        self.tls.fingerprint.as_ref()?.ja4.as_deref()
    }

    /// Expected JA4 for a resumed TLS 1.3 handshake, if captured.
    pub fn expected_resumed_ja4(&self) -> Option<&str> {
        self.tls.fingerprint.as_ref()?.resumed_ja4.as_deref()
    }

    /// Expected Akamai H2 fingerprint, if specified. Returns the base
    /// profile's expectation (typically the Windows form). For
    /// platform-specific expectations, use [`Self::expected_h2_fingerprint_for`].
    pub fn expected_h2_fingerprint(&self) -> Option<&str> {
        self.h2.fingerprint.as_ref()?.akamai.as_deref()
    }

    /// Expected Akamai H2 fingerprint for a specific platform key.
    /// Falls back to the base expectation when no override is declared.
    pub fn expected_h2_fingerprint_for(&self, platform: crate::profile::Platform) -> Option<&str> {
        if let Some(p) = self.h2.platforms.get(platform.identity_key()) {
            if let Some(ref fp) = p.fingerprint {
                if let Some(ref s) = fp.akamai {
                    return Some(s.as_str());
                }
            }
        }
        self.h2.fingerprint.as_ref()?.akamai.as_deref()
    }
}

impl H2Profile {
    /// Resolve this profile for a given platform. When a `[h2.platforms.X]`
    /// override exists, return a new H2Profile with the override applied
    /// (per-field `Some` wins; names in `omit_settings` clear the matching
    /// base field). Returns `self.clone()` when no override is declared.
    pub fn resolve_for_platform(
        &self,
        platform: crate::profile::Platform,
    ) -> Result<H2Profile, crate::Error> {
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
                    return Err(crate::Error::Config(format!(
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
mod tests {
    use super::*;
    use crate::profile::{Browser, Platform, ProfileRegistry};

    fn chrome_h2(version: Browser) -> H2Profile {
        ProfileRegistry::builtin()
            .get_browser(version)
            .expect("built-in profile")
            .h2
            .clone()
    }

    #[test]
    fn unknown_omit_settings_name_is_rejected() {
        let mut h2 = chrome_h2(Browser::Chrome147);
        let over = H2PlatformOverride {
            omit_settings: vec!["not_a_setting".into()],
            ..Default::default()
        };
        h2.platforms
            .insert(Platform::Windows.identity_key().to_string(), over);
        assert!(
            h2.resolve_for_platform(Platform::Windows).is_err(),
            "a bogus omit_settings name was silently ignored instead of rejected"
        );
    }

    #[test]
    fn builtin_platform_overrides_resolve_ok() {
        // Chrome 145 macOS override drops max_concurrent_streams + unknown_setting8.
        let h2 = chrome_h2(Browser::Chrome145);
        assert!(h2.resolve_for_platform(Platform::MacOS).is_ok());
    }
}
