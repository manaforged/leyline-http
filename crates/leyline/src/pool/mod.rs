#![forbid(unsafe_code)]

mod config;
mod connect;
pub(crate) mod h1;
mod liveness;
#[expect(
    clippy::module_inception,
    reason = "pool::pool is the pool engine; the parent module is the public facade"
)]
mod pool;
mod prepared;
mod route;
mod send;
mod types;

pub(crate) use config::{Io, PoolConfig};
pub use connect::checkout_handle;
#[cfg(feature = "http3")]
pub(crate) use connect::{H3Target, checkout_h3_handle};
#[cfg(feature = "websocket")]
pub(crate) use h1::upgrade_on_stream;
pub(crate) use liveness::checkout_live_h2;
pub use liveness::{DEFAULT_H2_PING_AFTER_IDLE, DEFAULT_H2_PING_TIMEOUT};
#[cfg(feature = "http3")]
pub(crate) use pool::OriginFlag;
pub use pool::{
    DEFAULT_IDLE_TIMEOUT, DEFAULT_MAX_CONNECTIONS, DEFAULT_MAX_H1_CONNS_PER_HOST,
    DEFAULT_MAX_H1_PENDING_PER_HOST, Pool,
};
pub(crate) use prepared::Prepared;
pub(crate) use route::{Route, send};
#[cfg(feature = "websocket")]
pub(crate) use send::open_extended_connect;
#[cfg(feature = "http3")]
pub(crate) use send::send_h3;
pub use types::{ConnectionStats, H1Slot};

#[cfg(test)]
mod tests;
