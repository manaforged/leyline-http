#![forbid(unsafe_code)]
pub(crate) mod anchor;
mod bare;
mod brand;
mod browser;
pub(crate) mod permutation;
mod platform;
pub(crate) mod preset;
mod registry;
mod types;

pub use anchor::HeaderAnchor;
pub(crate) use anchor::infer_anchor;
pub use brand::ChromiumBrand;
pub use browser::{Browser, Family};
pub use platform::Platform;
pub use preset::{HeaderStyle, Preset};
pub use registry::{ProfileError, ProfileRegistry};
pub use types::*;
