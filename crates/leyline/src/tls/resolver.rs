//! Pluggable DNS resolution for the TLS connector.
//!
//! The [`Resolver`] trait lets callers swap out address resolution for
//! testing (mock resolvers), host overrides (e.g. a `/etc/hosts`-style
//! map), or alternate backends (e.g. DoH). The default implementation,
//! [`SystemResolver`], wraps [`std::net::ToSocketAddrs`] inside
//! [`tokio::task::spawn_blocking`] so blocking `getaddrinfo(3)` calls
//! do not stall the runtime.
//!
//! The trait is object-safe and uses a manually-boxed future rather
//! than pulling in `async_trait`; Leyline tries to keep its direct
//! dependency count small.
//!
//! ```no_run
//! use std::io;
//! use std::net::SocketAddr;
//! use std::pin::Pin;
//! use std::future::Future;
//! use leyline::tls::{ResolveFuture, Resolver};
//!
//! struct LoopbackResolver;
//!
//! impl Resolver for LoopbackResolver {
//!     fn resolve<'a>(&'a self, _host: &'a str, port: u16) -> ResolveFuture<'a> {
//!         Box::pin(async move {
//!             Ok(vec![SocketAddr::from(([127, 0, 0, 1], port))])
//!         })
//!     }
//! }
//! ```

use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::pin::Pin;

/// A boxed future returned by [`Resolver::resolve`].
pub type ResolveFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<SocketAddr>, io::Error>> + Send + 'a>>;

/// Asynchronous host-to-`SocketAddr` resolver.
///
/// Implementations **must** return every address they know about for
/// `host:port`. Happy Eyeballs (RFC 8305) then interleaves the result
/// by address family and races TCP connect attempts. Returning only
/// the first hit defeats the dual-stack race.
pub trait Resolver: Send + Sync + 'static {
    /// Resolve `host` and `port` to a list of socket addresses.
    ///
    /// Implementations should return both `A` and `AAAA` records when
    /// available — the caller interleaves them.
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a>;
}

/// Default resolver: blocking `getaddrinfo(3)` off-thread via
/// [`tokio::task::spawn_blocking`].
///
/// This is what [`FingerprintConnector`] uses when no resolver is
/// provided. It honours `/etc/hosts`, `/etc/nsswitch.conf`, and every
/// other libc resolver knob — exactly the behaviour most production
/// callers want.
///
/// [`FingerprintConnector`]: crate::tls::FingerprintConnector
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemResolver;

impl Resolver for SystemResolver {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a> {
        // Fast path for IP literals: parsing avoids a `getaddrinfo(3)` call entirely, so we
        // do NOT pay a `spawn_blocking` hop (and the tokio blocking-pool thread it spawns).
        // Under connection churn at concurrency this otherwise inflates the blocking pool to
        // a dozen-plus threads, each costing a stack and a fresh glibc malloc arena.
        if let Ok(ip) = host.parse::<IpAddr>() {
            let addr = SocketAddr::new(ip, port);
            return Box::pin(async move { Ok(vec![addr]) });
        }
        let host = host.to_owned();
        Box::pin(async move {
            let addr = format!("{host}:{port}");
            tokio::task::spawn_blocking(move || {
                addr.to_socket_addrs().map(|it| it.collect::<Vec<_>>())
            })
            .await
            .map_err(io::Error::other)?
        })
    }
}

#[cfg(test)]
mod tests;
