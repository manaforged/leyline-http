#![forbid(unsafe_code)]

mod connect;
pub(crate) mod h1;
mod liveness;
#[expect(
    clippy::module_inception,
    reason = "pool::pool is the pool engine; the parent module is the public facade"
)]
mod pool;
mod send;
mod types;

pub use connect::checkout_handle;
#[cfg(feature = "http3")]
pub(crate) use connect::{H3Target, checkout_h3_handle};
#[cfg(feature = "bench-internals")]
pub use h1::H1Response;
#[cfg(feature = "websocket")]
pub(crate) use h1::upgrade_on_stream;
pub use h1::{H1Body, H1PooledError, H1ResponseBody, H1Target, send_request_h1_pooled};
pub(crate) use liveness::checkout_live_h2;
pub use liveness::{DEFAULT_H2_PING_AFTER_IDLE, DEFAULT_H2_PING_TIMEOUT};
#[cfg(feature = "http3")]
pub(crate) use pool::DEFAULT_MAX_ALT_SVC_ORIGINS;
pub use pool::{
    DEFAULT_IDLE_TIMEOUT, DEFAULT_MAX_CONNECTIONS, DEFAULT_MAX_H1_CONNS_PER_HOST, Pool,
};
pub(crate) use send::send_request;
#[cfg(feature = "http3")]
pub(crate) use send::{H3Request, send_request_h3_pooled};
pub use types::{H1Slot, PoolStats, TlsInfo};

use types::{PoolKey, Transport};

pub(crate) fn make_key(
    scheme: &str,
    host: &str,
    port: u16,
    proxy: Option<&str>,
    transport: Transport,
) -> PoolKey {
    PoolKey {
        scheme: scheme.to_string(),
        host: host.to_string(),
        port,
        proxy: proxy.map(|s| s.to_string()),
        transport,
    }
}

#[cfg(test)]
mod tests;
