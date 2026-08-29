//! HTTP/3 over QUIC with browser-profiled fingerprinting.
//!
//! Controls QUIC transport parameters, HTTP/3 SETTINGS, QPACK configuration,
//! and stream creation ordering from the selected browser profile. A pooled
//! connection survives between requests via a driver task and multiplexes
//! request streams ([`pool`]).

#![forbid(unsafe_code)]
// This module must stay free of `unsafe`; memory-unsafe code is confined to
// leyline-bssl* (FFI) and leyline's tcp/tls platform bridges (reviewed there).
mod config;
mod connection;
mod pool;

pub use config::H3Config;
pub use connection::H3Response;
pub(crate) use pool::{
    H3Client, H3DriverTask, H3RequestBodyStream, H3RespBody, H3ResponseParts, open_fresh_h3,
};
