#![forbid(unsafe_code)]
pub mod anchor;
mod bare;
mod brand;
mod browser;
pub(crate) mod permutation;
mod platform;
pub mod preset;
mod registry;
mod types;

pub use anchor::{HeaderAnchor, infer_anchor};
pub use brand::{BrandOverlay, BrandOverlayError, ChromiumBrand};
pub use browser::{Browser, Family};
pub use platform::Platform;
pub use preset::Preset;
pub use registry::{ProfileError, ProfileRegistry};
pub use types::*;
