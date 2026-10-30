use std::collections::HashMap;

use serde::Deserialize;

use crate::h2::config::SettingId;
use crate::profile::H3Profile;
use crate::profile::preset::HeaderStyle;
use crate::{Cause, Error};

#[cfg_attr(
    feature = "bench-internals",
    expect(
        missing_docs,
        reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
    )
)]
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

#[cfg_attr(
    feature = "bench-internals",
    expect(
        missing_docs,
        reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
    )
)]
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

#[cfg_attr(
    feature = "bench-internals",
    expect(
        missing_docs,
        reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
    )
)]
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
    #[serde(default = "default_grease")]
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

const fn default_grease() -> bool {
    true
}

const fn default_true() -> bool {
    true
}

#[cfg_attr(
    feature = "bench-internals",
    expect(
        missing_docs,
        reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
    )
)]
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
        not(any(test, feature = "bench-internals")),
        expect(
            dead_code,
            reason = "the TOML schema keeps this key; only tests and bench-internals read it"
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

#[cfg_attr(
    feature = "bench-internals",
    expect(
        missing_docs,
        reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
    )
)]
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
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
    pub default_priority: Option<H2PriorityProfile>,
    #[serde(default)]
    pub fingerprint: Option<H2Fingerprint>,
    #[serde(default)]
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

#[cfg_attr(
    feature = "bench-internals",
    expect(
        missing_docs,
        reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
    )
)]
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
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
    #[serde(default)]
    pub omit_settings: Vec<String>,
    #[serde(default)]
    pub pseudo_order: Option<Vec<String>>,
    #[serde(default)]
    pub settings_order: Option<Vec<String>>,
    #[cfg_attr(
        not(feature = "bench-internals"),
        expect(
            dead_code,
            reason = "the TOML schema keeps this key; only bench-internals reads it"
        )
    )]
    #[serde(default)]
    pub fingerprint: Option<H2Fingerprint>,
}

#[cfg_attr(
    feature = "bench-internals",
    expect(
        missing_docs,
        reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
    )
)]
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

#[cfg_attr(
    feature = "bench-internals",
    expect(
        missing_docs,
        reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
    )
)]
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

fn invalid_profile(why: impl std::fmt::Display) -> crate::Error {
    crate::Error::new(crate::Cause::Config).with_message(format!("invalid profile: {why}"))
}

impl BrowserProfile {
    pub fn from_toml(toml_str: &str) -> crate::Result<Self> {
        let mut profile: Self = toml::from_str(toml_str).map_err(invalid_profile)?;
        profile.derive_sec_ch_ua();
        profile.source = Some(source::ProfileSource::new(toml_str));
        profile
            .tls
            .validate(false)
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

    bench_pub! {
        fn load_warnings(&self) -> Vec<String> {
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
    }

    bench_pub! {
        fn identity_for(&self, platform: crate::profile::Platform) -> Option<&BrowserHeaders> {
            self.identity.get(platform.identity_key())
        }
    }

    pub(crate) fn source(&self) -> Option<&str> {
        self.source.as_ref().map(source::ProfileSource::as_str)
    }
}

impl H2Profile {
    bench_pub! {
        fn resolve_for_platform(
            &self,
            platform: crate::profile::Platform,
        ) -> Result<H2Profile, Error> {
            let Some(over) = self.platforms.get(platform.identity_key()) else {
                return Ok(self.clone());
            };
            let mut out = self.clone();
            let config = |why: String| Error::new(Cause::Config).with_message(why);

            for id in SettingId::all() {
                if let Some(v) = over.setting(id) {
                    out.set_setting(id, Some(v)).map_err(config)?;
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
                let Some(id) = setting_by_field(name) else {
                    return Err(config(format!(
                        "unknown name in [h2.platforms.{}].omit_settings: {name:?}",
                        platform.identity_key()
                    )));
                };
                out.set_setting(id, None).map_err(config)?;
            }

            Ok(out)
        }
    }

    pub(crate) fn set_setting(&mut self, id: SettingId, value: Option<u32>) -> Result<(), String> {
        match id {
            SettingId::HeaderTableSize => self.header_table_size = value,
            SettingId::EnablePush => {
                self.enable_push = match value {
                    None => None,
                    Some(0) => Some(false),
                    Some(1) => Some(true),
                    Some(other) => return Err(format!("ENABLE_PUSH value {other} is not 0 or 1")),
                };
            }
            SettingId::MaxConcurrentStreams => self.max_concurrent_streams = value,
            SettingId::InitialWindowSize => self.initial_stream_window_size = value,
            SettingId::MaxFrameSize => self.max_frame_size = value,
            SettingId::MaxHeaderListSize => self.max_header_list_size = value,
            SettingId::Unknown8 => self.unknown_setting8 = value,
            SettingId::Unknown9 => self.unknown_setting9 = value,
        }
        Ok(())
    }
}

fn setting_by_field(name: &str) -> Option<SettingId> {
    Some(match name {
        "header_table_size" => SettingId::HeaderTableSize,
        "enable_push" => SettingId::EnablePush,
        "max_concurrent_streams" => SettingId::MaxConcurrentStreams,
        "initial_stream_window_size" => SettingId::InitialWindowSize,
        "max_frame_size" => SettingId::MaxFrameSize,
        "max_header_list_size" => SettingId::MaxHeaderListSize,
        "unknown_setting8" => SettingId::Unknown8,
        "unknown_setting9" => SettingId::Unknown9,
        _ => return None,
    })
}

macro_rules! h2_setting_getter {
    ($($ty:ty),+) => {
        $(
            impl $ty {
                pub(crate) fn setting(&self, id: SettingId) -> Option<u32> {
                    match id {
                        SettingId::HeaderTableSize => self.header_table_size,
                        SettingId::EnablePush => self.enable_push.map(u32::from),
                        SettingId::MaxConcurrentStreams => self.max_concurrent_streams,
                        SettingId::InitialWindowSize => self.initial_stream_window_size,
                        SettingId::MaxFrameSize => self.max_frame_size,
                        SettingId::MaxHeaderListSize => self.max_header_list_size,
                        SettingId::Unknown8 => self.unknown_setting8,
                        SettingId::Unknown9 => self.unknown_setting9,
                    }
                }
            }
        )+
    };
}

h2_setting_getter!(H2Profile, H2PlatformOverride);

mod source;

#[cfg(test)]
mod tests;
