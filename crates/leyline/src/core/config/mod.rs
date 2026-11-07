mod compression;
mod host;
mod host_gate;
mod host_limits;
mod origin;
mod proxy;
mod redirect;
mod websocket;

pub(crate) use compression::{BodyRules, Codings, MAX_CODINGS};
pub(crate) use host_gate::{HostGates, HostPass};
pub use host_limits::HostLimits;
pub(crate) use host_limits::HostStats;
pub(crate) use origin::Origin;
pub use proxy::Proxy;
pub(crate) use proxy::ProxyRule;
pub use redirect::Redirect;
pub(crate) use websocket::WebSocketOptions;

#[cfg(test)]
use crate::net::ProxyUrl;

#[cfg(test)]
mod tests;
