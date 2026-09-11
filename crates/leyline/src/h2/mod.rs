#![forbid(unsafe_code)]
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
