mod compression;
mod dns;
mod host;
mod pool;
mod proxy;
mod redirect;
mod socket;
mod timeout;
mod websocket;

pub use compression::CompressionConfig;
pub(crate) use compression::DEFAULT_MAX_BODY_SIZE;
pub use dns::DnsConfig;
pub use pool::PoolConfig;
pub use proxy::{NoProxy, ProxyConfig, ProxyRule, ProxyUrl};
pub use redirect::{RedirectAction, RedirectAttempt, RedirectPolicy};
pub use socket::SocketConfig;
pub use timeout::TimeoutConfig;
pub use websocket::WebSocketConfig;

#[cfg(test)]
mod tests;
