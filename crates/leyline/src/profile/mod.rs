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

pub(crate) use anchor::HeaderPosition;
pub(crate) use anchor::infer_anchor;
pub(crate) use brand::ChromiumBrand;
pub(crate) use browser::{Browser, Family};
pub(crate) use extension::{ALPN, SERVER_NAME};
pub use fingerprint::FingerprintSpec;
pub use h3::H3Profile;
#[cfg(any(feature = "http3", feature = "bench-internals"))]
pub use h3::{
    H3ConnectionIdLength, H3CryptoReorder, H3CryptoSplit, H3Grease, H3Order, H3Setting,
    H3TransportParam, H3VersionGrease, H3VersionInformation,
};
pub(crate) use platform::Platform;
pub use preset::HeaderStyle;
pub(crate) use preset::RequestKind;
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub use registry::ProfileRegistry;
#[cfg(not(feature = "bench-internals"))]
pub(crate) use registry::ProfileRegistry;
pub(crate) use types::BrowserProfile;
pub use types::{BrowserHeaders, H2Profile, TlsProfile};
#[cfg(feature = "bench-internals")]
pub use types::{
    H2Fingerprint, H2PlatformOverride, H2PriorityProfile, ProfileMeta, TlsFingerprint,
};
