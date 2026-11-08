#![forbid(unsafe_code)]
macro_rules! string_id_serde {
    ($ty:ty) => {
        impl serde::Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.id())
            }
        }

        impl<'de> serde::Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let id = String::deserialize(deserializer)?;
                id.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

pub(crate) mod anchor;
mod bare;
pub(crate) mod brand;
pub(crate) mod browser;
mod coding;
mod extension;
mod fingerprint;
pub(crate) mod h2;
mod h3;
mod identity;
pub(crate) mod languages;
pub(crate) mod permutation;
pub(crate) mod platform;
pub(crate) mod preset;
mod registry;
mod tcp;
pub(crate) mod types;

pub(crate) use anchor::HeaderPosition;
pub(crate) use anchor::infer_anchor;
pub(crate) use brand::ChromiumBrand;
pub(crate) use browser::{Browser, Family};
pub use coding::Encoding;
pub(crate) use extension::Carrier;
pub use fingerprint::{FingerprintSpec, akamai_fingerprint};
pub(crate) use fingerprint::{JA3_POINT_FORMATS, JA3_RECORD_VERSION, JA4_TCP};
pub(crate) use h2::{DEFAULT_WINDOW_SIZE, NO_PRIORITY_FRAMES, parse_pseudo_order};
pub use h2::{H2Settings, PseudoOrder, SettingId};
pub use h3::H3Profile;
#[cfg(feature = "http3")]
pub use h3::{
    H3ConnectionIdLength, H3CryptoReorder, H3CryptoSplit, H3Grease, H3Order, H3Setting,
    H3TransportParam, H3VersionGrease, H3VersionInformation,
};
pub(crate) use platform::Platform;
pub use preset::HeaderStyle;
pub(crate) use preset::RequestKind;
pub use registry::ProfileRegistry;
pub use tcp::TcpProfile;
pub(crate) use types::CertCompression;
pub use types::TlsMinVersion;
pub use types::{BrowserHeaders, BrowserProfile, H2Profile, TlsProfile};
