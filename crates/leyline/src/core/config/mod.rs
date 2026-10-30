mod compression;
mod dns;
mod host;
mod host_gate;
mod host_limits;
mod origin;
mod pool;
mod proxy;
mod redirect;
mod socket;
mod timeout;
mod websocket;

pub(crate) use compression::BodyRules;
pub(crate) use compression::{DEFAULT_MAX_BODY_SIZE, DEFAULT_MAX_HEADER_LIST_BYTES};
pub(crate) use dns::DnsConfig;
#[cfg(feature = "html")]
pub(crate) use host::normalize_host;
pub use host_limits::HostLimits;
pub(crate) use host_limits::HostStats;
pub(crate) use host_limits::{HostPass, Origin};
pub(crate) use pool::PoolConfig;
pub use proxy::Proxy;
pub(crate) use proxy::ProxyRule;
pub(crate) use proxy::ProxyUrl;
pub use redirect::Redirect;
pub(crate) use socket::SocketOptions;
pub use timeout::Timeout;
pub(crate) use websocket::WebSocketOptions;

#[cfg(test)]
mod tests;
