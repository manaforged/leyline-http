use std::io;
use std::net::SocketAddr;

use tokio::net::UdpSocket;

use crate::net::ProxyUrl;
use crate::net::proxy::SOCKS_FEATURE_OFF;
#[cfg(feature = "socks")]
use crate::net::proxy::socks5::udp::Socks5Udp;
use crate::quic::connection::resolve_peer;
use crate::quic::error::QuicError;
use crate::tls::FingerprintConnector;

pub(crate) enum DatagramTransport {
    Direct(UdpSocket),
    #[cfg(feature = "socks")]
    Socks5(Socks5Udp),
}

impl DatagramTransport {
    pub(crate) async fn open(
        connector: &FingerprintConnector,
        host: &str,
        port: u16,
        proxy: Option<&ProxyUrl>,
    ) -> Result<(Self, SocketAddr), QuicError> {
        match proxy {
            None => Self::open_direct(connector, host, port).await,
            Some(proxy) => {
                if !proxy.scheme().is_socks() {
                    return Err(QuicError::Proxy(format!(
                        "HTTP/3 cannot use a `{}` proxy: QUIC needs a socks5:// or socks5h:// \
                         proxy with UDP ASSOCIATE",
                        proxy.url().scheme()
                    )));
                }
                Self::open_socks5(connector, host, port, proxy.url()).await
            }
        }
    }

    async fn open_direct(
        connector: &FingerprintConnector,
        host: &str,
        port: u16,
    ) -> Result<(Self, SocketAddr), QuicError> {
        let peer_addr = resolve_peer(connector.dialer().resolver(), host, port).await?;
        let socket = UdpSocket::bind(crate::util::unspecified_for(peer_addr))
            .await
            .map_err(|e| QuicError::Socket(format!("udp bind: {e}")))?;
        socket
            .connect(peer_addr)
            .await
            .map_err(|e| QuicError::Socket(format!("udp connect: {e}")))?;
        Ok((Self::Direct(socket), peer_addr))
    }

    #[cfg(feature = "socks")]
    async fn open_socks5(
        connector: &FingerprintConnector,
        host: &str,
        port: u16,
        proxy: &url::Url,
    ) -> Result<(Self, SocketAddr), QuicError> {
        let (relay, relay_addr) = Socks5Udp::open(connector.dialer(), proxy, host, port)
            .await
            .map_err(|e| QuicError::Proxy(format!("h3 proxy: {e}")))?;
        Ok((Self::Socks5(relay), relay_addr))
    }

    #[cfg(not(feature = "socks"))]
    async fn open_socks5(
        _: &FingerprintConnector,
        _: &str,
        _: u16,
        _: &url::Url,
    ) -> Result<(Self, SocketAddr), QuicError> {
        Err(QuicError::Proxy(SOCKS_FEATURE_OFF.to_string()))
    }

    pub(crate) fn local_addr(&self) -> io::Result<SocketAddr> {
        match self {
            Self::Direct(socket) => socket.local_addr(),
            #[cfg(feature = "socks")]
            Self::Socks5(relay) => relay.local_addr(),
        }
    }

    pub(crate) async fn send(&self, datagram: &[u8]) -> io::Result<usize> {
        match self {
            Self::Direct(socket) => socket.send(datagram).await,
            #[cfg(feature = "socks")]
            Self::Socks5(relay) => relay.send(datagram).await,
        }
    }

    pub(crate) async fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Direct(socket) => socket.recv(buf).await,
            #[cfg(feature = "socks")]
            Self::Socks5(relay) => relay.recv(buf).await,
        }
    }
}

const PROXY_NO_UDP: &str = "HTTP/3 needs a socks5:// or socks5h:// proxy with UDP ASSOCIATE; \
                            http and https proxies cannot carry QUIC, use Auto or Http2";

pub(crate) fn h3_proxy_blocker(proxy: Option<&ProxyUrl>) -> Option<&'static str> {
    let udp = proxy?.scheme().is_socks();
    match (udp, cfg!(feature = "socks")) {
        (true, true) => None,
        (true, false) => Some(SOCKS_FEATURE_OFF),
        (false, _) => Some(PROXY_NO_UDP),
    }
}

pub(crate) fn proxy_carries_h3(proxy: Option<&ProxyUrl>) -> bool {
    h3_proxy_blocker(proxy).is_none()
}
