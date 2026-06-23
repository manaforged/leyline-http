//! HTTP/3 over QUIC with browser-profiled fingerprinting.
//!
//! Controls QUIC transport parameters, HTTP/3 SETTINGS, QPACK configuration,
//! and stream creation ordering from the selected browser profile. A pooled
//! connection survives between requests via a driver task and multiplexes
//! request streams ([`pool`]).

mod config;
mod connection;
mod pool;

pub use config::H3Config;
pub use connection::H3Response;
pub(crate) use pool::{
    open_fresh_h3, H3Client, H3DriverTask, H3RequestBodyStream, H3RespBody, H3ResponseParts,
};
