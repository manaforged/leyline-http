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

pub use compression::CompressionConfig;
pub(crate) use compression::{DEFAULT_MAX_BODY_SIZE, DEFAULT_MAX_HEADER_LIST_BYTES};
pub use dns::DnsConfig;
#[cfg(feature = "html")]
pub(crate) use host::normalize_host;
pub use host_limits::{HostLimits, HostStats};
pub(crate) use host_limits::{HostPass, Origin};
pub use pool::PoolConfig;
pub use proxy::{NoProxy, ProxyConfig, ProxyRule, ProxyUrl};
pub use redirect::{RedirectAction, RedirectAttempt, RedirectPolicy};
pub use socket::SocketConfig;
pub use timeout::TimeoutConfig;
pub use websocket::WebSocketConfig;

#[cfg(test)]
mod tests;
