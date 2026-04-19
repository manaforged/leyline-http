//! Browser/platform/preset profile definitions and TOML loader.
//!
//! Profiles are declarative TOML files embedded at compile time. Adding a new
//! browser version means creating a TOML file — no core code changes needed.

/// Anchor slots for caller-controlled positional header injection.
pub mod anchor;
mod brand;
mod browser;
mod platform;
/// Request preset types and header builder.
pub mod preset;
mod registry;
mod types;

pub use anchor::{infer_anchor, HeaderAnchor};
pub use brand::{BrandOverlay, BrandOverlayError, ChromiumBrand};
pub use browser::{Browser, ALL_BROWSERS, PROFILE_COUNT};
pub use platform::Platform;
pub use preset::Preset;
pub use registry::ProfileRegistry;
pub use types::*;
