#![forbid(unsafe_code)]
pub(crate) mod client;
pub mod codec;
pub mod config;
pub mod connection;
pub mod error;
pub mod frame;
pub mod hpack;
pub mod stream_state;

#[cfg(feature = "bench-internals")]
pub use crate::core::{ErrorBudget, ResponseMode};
pub use client::start;
#[cfg(feature = "bench-internals")]
pub use client::{H2Client, H2ResponseEx, Head, RequestBody, ResponseBody};
pub use config::H2Config;
#[cfg(feature = "bench-internals")]
pub use config::PriorityParams;
pub use error::{ErrorCode, H2Error};
