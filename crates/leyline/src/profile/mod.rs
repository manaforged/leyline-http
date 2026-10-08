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
mod extension;
mod fingerprint;
mod h3;
mod identity;
pub(crate) mod languages;
pub(crate) mod permutation;
pub(crate) mod platform;
pub(crate) mod preset;
mod registry;
pub(crate) mod types;

pub(crate) use anchor::HeaderAnchor;
pub(crate) use anchor::infer_anchor;
pub(crate) use brand::ChromiumBrand;
pub(crate) use browser::{Browser, Family};
pub use fingerprint::FingerprintSpec;
pub use h3::{
    H3ConnectionIdLength, H3CryptoReorder, H3CryptoSplit, H3Grease, H3Order, H3Profile, H3Setting,
    H3TransportParam, H3VersionGrease, H3VersionInformation,
};
pub(crate) use platform::Platform;
pub use preset::HeaderStyle;
pub(crate) use preset::Preset;
pub use registry::{ProfileError, ProfileRegistry};
pub(crate) use types::BrowserProfile;
pub use types::{
    H2Fingerprint, H2PlatformOverride, H2PriorityProfile, H2Profile, PlatformIdentity, ProfileMeta,
    TlsFingerprint, TlsProfile,
};
