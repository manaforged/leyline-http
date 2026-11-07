use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::pin::Pin;
use std::sync::Arc;

use crate::util::coalesce::Coalesce;

pub(crate) type ResolveFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<SocketAddr>, io::Error>> + Send + 'a>>;

/// Resolves a host name and port to socket addresses for new connections.
pub trait Resolver: Send + Sync + 'static {
    /// Returns the socket addresses for `host` and `port`.
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SocketAddr>, io::Error>> + Send + 'a>>;
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SystemResolver;

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

type Lookup = Result<Vec<SocketAddr>, Arc<io::Error>>;

/// Shares one lookup between concurrent connections to the same host and port.
pub(crate) struct Coalesced {
    inner: Arc<dyn Resolver>,
    running: Coalesce<(String, u16), Lookup>,
}

impl Coalesced {
    pub(crate) fn new(inner: Arc<dyn Resolver>) -> Self {
        Self {
            inner,
            running: Coalesce::default(),
        }
    }
}

impl Resolver for Coalesced {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a> {
        let inner = Arc::clone(&self.inner);
        let owned = host.to_owned();
        let lookup = self.running.join_or_spawn(
            &(owned.clone(), port),
            move || async move { inner.resolve(&owned, port).await.map_err(Arc::new) },
            |e| Err(Arc::new(io::Error::other(e))),
        );
        Box::pin(async move {
            lookup
                .await
                .map_err(|e| io::Error::new(e.kind(), e.to_string()))
        })
    }
}

#[cfg(test)]
mod tests;
