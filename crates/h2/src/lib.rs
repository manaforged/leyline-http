//! leyline-h2 — HTTP/2 with native fingerprint control.
//!
//! A ground-up RFC 9113 implementation designed for TLS fingerprinting.
//! SETTINGS ordering, pseudo-header ordering, and connection preface
//! timing are first-class concepts, not afterthoughts.

pub mod client;
pub mod codec;
pub mod config;
pub mod connection;
pub mod error;
pub mod frame;
pub mod hpack;
pub mod stream_state;

pub use client::{DriverTask, H2Client, H2ConnectStream};
pub use config::{H2Config, PriorityParams, PseudoOrder, SettingId, SETTINGS_ENABLE_CONNECT_PROTOCOL};
pub use error::H2Error;
pub use stream_state::{StreamEvent, StreamState, StreamStateError};
