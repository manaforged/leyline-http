use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpStream;

use super::happy_eyeballs::happy_eyeballs_connect;
use super::resolver::{Resolver, SystemResolver};
use super::socket::configure;
use super::{NetError, SocketOptions};
use crate::profile::TcpProfile;
use crate::trace;

/// Opens TCP connections: resolves the host, races addresses, and sets up each socket.
#[derive(Clone)]
pub(crate) struct Dialer {
    resolver: Arc<dyn Resolver>,
    tcp: TcpProfile,
    socket: SocketOptions,
}

impl Dialer {
    pub(crate) fn new(tcp: TcpProfile) -> Self {
        Self {
            resolver: Arc::new(SystemResolver),
            tcp,
            socket: SocketOptions::default(),
        }
    }

    pub(crate) fn with_resolver(mut self, resolver: Arc<dyn Resolver>) -> Self {
        self.resolver = resolver;
        self
    }

    pub(crate) fn with_socket(mut self, socket: SocketOptions) -> Self {
        self.socket = socket;
        self
    }

    pub(crate) fn with_tcp(mut self, tcp: TcpProfile) -> Self {
        self.tcp = tcp;
        self
    }

    pub(crate) fn tcp_profile(&self) -> &TcpProfile {
        &self.tcp
    }

    #[cfg(feature = "http3")]
    pub(crate) fn resolver(&self) -> &dyn Resolver {
        self.resolver.as_ref()
    }

    pub(crate) async fn dial(&self, host: &str, port: u16) -> Result<TcpStream, NetError> {
        let addrs = self
            .resolver
            .resolve(host, port)
            .await
            .map_err(NetError::Dns)?;
        trace::dns(host, port, addrs.len());

        if addrs.is_empty() {
            return Err(NetError::Dns(io::Error::new(
                io::ErrorKind::NotFound,
                "no addresses resolved",
            )));
        }

        let race = self.socket.happy_eyeballs.unwrap_or_default();
        let (tcp_stream, _addr) = happy_eyeballs_connect(addrs, race, |peer| {
            let tcp = self.tcp.clone();
            let socket = self.socket.clone();
            async move { connect_one(peer, &tcp, &socket).await }
        })
        .await
        .map_err(NetError::TcpConnect)?;
        trace::connect(host, port, false);
        Ok(tcp_stream)
    }
}

async fn connect_one(
    peer: SocketAddr,
    tcp: &TcpProfile,
    options: &SocketOptions,
) -> io::Result<TcpStream> {
    let domain = if peer.is_ipv6() {
        socket2::Domain::IPV6
    } else {
        socket2::Domain::IPV4
    };
    let socket = socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))?;
    configure(&socket, peer, tcp, options)?;
    socket.set_nonblocking(true)?;

    match socket.connect(&peer.into()) {
        Ok(()) => {}
        Err(e) if nonblocking_connect_started(&e) => {}
        Err(e) => return Err(e),
    }
    let std_stream: std::net::TcpStream = socket.into();
    let tcp_stream = TcpStream::from_std(std_stream)?;

    tcp_stream.writable().await?;
    if let Some(e) = tcp_stream.take_error()? {
        return Err(e);
    }
    Ok(tcp_stream)
}

fn nonblocking_connect_started(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::WouldBlock
        || error.raw_os_error() == Some(libc::EINPROGRESS)
        || windows_nonblocking_connect_started(error)
}

#[cfg(windows)]
fn windows_nonblocking_connect_started(error: &io::Error) -> bool {
    const WSAEWOULDBLOCK: i32 = 10035;
    const WSAEINPROGRESS: i32 = 10036;
    matches!(
        error.raw_os_error(),
        Some(WSAEWOULDBLOCK) | Some(WSAEINPROGRESS)
    )
}

#[cfg(not(windows))]
fn windows_nonblocking_connect_started(_error: &io::Error) -> bool {
    false
}

#[cfg(test)]
mod tests;
