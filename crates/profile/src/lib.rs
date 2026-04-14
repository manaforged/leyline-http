//! Browser/platform/preset profile definitions and TOML loader.
//!
//! Profiles are declarative TOML files embedded at compile time. Adding a new
//! browser version means creating a TOML file — no core code changes needed.

mod browser;
mod platform;
/// Request preset types and header builder.
pub mod preset;
mod registry;
mod types;

pub use browser::{Browser, ALL_BROWSERS, PROFILE_COUNT};
pub use platform::Platform;
pub use preset::Preset;
pub use registry::ProfileRegistry;
pub use types::*;
