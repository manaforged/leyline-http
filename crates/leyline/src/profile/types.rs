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
                    )))
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
