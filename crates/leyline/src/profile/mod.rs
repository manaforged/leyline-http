//! Browser/platform/preset profile definitions and TOML loader.

#![forbid(unsafe_code)]
/// Anchor slots for caller-controlled positional header injection.
pub mod anchor;
mod bare;
mod brand;
mod browser;
pub(crate) mod permutation;
mod platform;
/// Request preset types and header builder.
pub mod preset;
mod registry;
mod types;

pub use anchor::{HeaderAnchor, infer_anchor};
pub use brand::{BrandOverlay, BrandOverlayError, ChromiumBrand};
pub use browser::{ALL_BROWSERS, Browser, Family, PROFILE_COUNT};
pub use platform::Platform;
pub use preset::Preset;
pub use registry::{ProfileError, ProfileRegistry};
pub use types::*;
