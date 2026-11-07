mod blocking;
mod dial;
mod dns;
mod error;
mod happy_eyeballs;
pub(crate) mod proxy;
mod resolver;
mod socket;

pub(crate) use blocking::{BlockingTcp, probing};
pub(crate) use dial::Dialer;
pub(crate) use dns::{DnsConfig, normalize_host};
pub use error::{NetError, ProxyReply};
pub(crate) use happy_eyeballs::HappyEyeballs;
pub use proxy::ProxyUrl;
pub use resolver::Resolver;
#[cfg(all(test, feature = "http3"))]
pub(crate) use resolver::SystemResolver;
pub(crate) use socket::SocketOptions;
