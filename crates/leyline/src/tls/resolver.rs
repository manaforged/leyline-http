//! Pluggable DNS resolution for the TLS connector.

use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::pin::Pin;

/// A boxed future returned by [`Resolver::resolve`].
pub type ResolveFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<SocketAddr>, io::Error>> + Send + 'a>>;

/// Asynchronous host-to-`SocketAddr` resolver.
pub trait Resolver: Send + Sync + 'static {
    /// Resolve `host` and `port` to a list of socket addresses.
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a>;
}

/// Default resolver: blocking `getaddrinfo(3)` off-thread via [`tokio::task::spawn_blocking`].
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemResolver;

impl Resolver for SystemResolver {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a> {
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
