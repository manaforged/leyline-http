#![forbid(unsafe_code)]
pub(crate) mod client;
pub mod codec;
pub mod config;
pub mod connection;
pub mod error;
pub mod frame;
pub mod hpack;
pub mod stream_state;

pub use client::start;
pub use config::H2Config;
#[cfg(any(test, feature = "bench-internals"))]
pub use error::ErrorCode;
pub use error::H2Error;
