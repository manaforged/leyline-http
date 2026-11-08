#![forbid(unsafe_code)]
mod client;
mod config;
mod connection;
mod error;
mod transport;
mod wire;

const TIMER_FALLBACK: std::time::Duration = std::time::Duration::from_secs(5);

pub(crate) use client::{H3Client, H3SendError, open_fresh_h3};
pub use config::H3Config;
pub(crate) use transport::{h3_proxy_blocker, proxy_carries_h3};
