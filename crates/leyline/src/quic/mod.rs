//! HTTP/3 over QUIC with browser-profiled fingerprinting.
//!
//! Controls QUIC transport parameters, HTTP/3 SETTINGS, QPACK configuration,
//! and stream creation ordering from the selected browser profile.

mod config;
mod connection;

pub use config::H3Config;
pub use connection::{H3Connection, H3Response};
