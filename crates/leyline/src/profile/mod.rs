#![forbid(unsafe_code)]
pub(crate) mod anchor;
mod bare;
mod brand;
mod browser;
mod extension;
mod fingerprint;
mod h3;
mod identity;
pub(crate) mod permutation;
mod platform;
pub(crate) mod preset;
mod registry;
mod types;

pub use anchor::HeaderAnchor;
pub(crate) use anchor::infer_anchor;
pub use brand::ChromiumBrand;
pub use browser::{Browser, Family};
pub use fingerprint::FingerprintSpec;
pub use h3::{
    H3ConnectionIdLength, H3Grease, H3Order, H3Profile, H3Setting, H3TransportParam,
    H3VersionGrease, H3VersionInformation,
};
pub(crate) use identity::resolve_identity;
pub use platform::Platform;
pub use preset::{HeaderStyle, Preset};
pub use registry::{ProfileError, ProfileRegistry};
pub use types::{
    BrowserProfile, H2Fingerprint, H2PlatformOverride, H2PriorityProfile, H2Profile,
    PlatformIdentity, ProfileMeta, TlsFingerprint, TlsProfile,
};
