use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::pin::Pin;

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

#[cfg(test)]
mod tests;
