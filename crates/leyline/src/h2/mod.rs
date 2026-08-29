//! leyline-h2 — HTTP/2 with native fingerprint control.
//!
//! A ground-up RFC 9113 implementation designed for TLS fingerprinting.
//! SETTINGS ordering, pseudo-header ordering, and connection preface
//! timing are first-class concepts, not afterthoughts.

#![forbid(unsafe_code)]
// This module must stay free of `unsafe`; memory-unsafe code is confined to
// leyline-bssl* (FFI) and leyline's tcp/tls platform bridges.
pub(crate) mod client;
pub mod codec;
pub mod config;
pub mod connection;
pub mod error;
pub mod frame;
pub mod hpack;
pub mod stream_state;

pub use client::{DriverTask, H2Client, H2ConnectStream, H2ResponseEx, RequestBody, ResponseBody};
pub use config::{
    H2Config, PriorityParams, PseudoOrder, SETTINGS_ENABLE_CONNECT_PROTOCOL, SettingId,
};
pub use error::{ErrorCode, H2Error};
pub use stream_state::{StreamEvent, StreamState, StreamStateError};
