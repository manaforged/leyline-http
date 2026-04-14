//! # Leyline
//!
//! Browser-accurate TLS fingerprinting for Rust.
//!
//! ```rust,ignore
//! let session = leyline::Session::chrome()?;
//! let resp = session.navigate("https://example.com").await?;
//! println!("{}", resp.text());
//! ```

// Core types
pub use leyline_core::{Error, RequestBuilder, Response, Result, Session, SessionBuilder};

// Profile types
pub use leyline_profile::{Browser, Platform, Preset, ALL_BROWSERS, PROFILE_COUNT};

// TCP fingerprinting
pub use leyline_tcp::TcpProfile;

// Cookies
pub use leyline_cookies::CookieJar;
