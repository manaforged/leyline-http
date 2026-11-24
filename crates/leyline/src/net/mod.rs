mod blocking;
mod dial;
mod dns;
pub(crate) mod dns_wire;
mod error;
mod happy_eyeballs;
mod outlier;
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
pub(crate) use resolver::{EchFuture, ResolveFuture};
pub(crate) use socket::SocketOptions;
#[cfg(all(feature = "http3", target_os = "linux"))]
pub(crate) use socket::try_send_udp_batch;
#[cfg(feature = "http3")]
pub(crate) use socket::{MAX_UDP_BATCH, bind_udp};
