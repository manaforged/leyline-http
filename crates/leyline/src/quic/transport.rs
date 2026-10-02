use std::io;
use std::net::SocketAddr;

use tokio::net::UdpSocket;

use crate::quic::connection::resolve_peer;
use crate::tls::FingerprintConnector;
#[cfg(feature = "socks")]
use crate::tls::proxy::socks5::udp::Socks5Udp;

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
        proxy: Option<&str>,
    ) -> Result<(Self, SocketAddr), String> {
        match proxy {
            None => Self::open_direct(connector, host, port).await,
            Some(proxy) => {
                let proxy =
                    url::Url::parse(proxy).map_err(|e| format!("invalid proxy URL: {e}"))?;
                if !carries_udp(proxy.scheme()) {
                    return Err(format!(
                        "HTTP/3 cannot use a `{}` proxy: QUIC needs a socks5:// or socks5h:// \
                         proxy with UDP ASSOCIATE",
                        proxy.scheme()
                    ));
                }
                Self::open_socks5(connector, host, port, &proxy).await
            }
        }
    }

    async fn open_direct(
        connector: &FingerprintConnector,
        host: &str,
        port: u16,
    ) -> Result<(Self, SocketAddr), String> {
        let peer_addr = resolve_peer(connector.resolver().as_ref(), host, port).await?;
        let socket = UdpSocket::bind(crate::util::unspecified_for(peer_addr))
            .await
            .map_err(|e| format!("udp bind: {e}"))?;
        socket
            .connect(peer_addr)
            .await
            .map_err(|e| format!("udp connect: {e}"))?;
        Ok((Self::Direct(socket), peer_addr))
    }

    #[cfg(feature = "socks")]
    async fn open_socks5(
        connector: &FingerprintConnector,
        host: &str,
        port: u16,
        proxy: &url::Url,
    ) -> Result<(Self, SocketAddr), String> {
        let (relay, relay_addr) = Socks5Udp::open(connector, proxy, host, port)
            .await
            .map_err(|e| format!("h3 proxy: {e}"))?;
        Ok((Self::Socks5(relay), relay_addr))
    }

    #[cfg(not(feature = "socks"))]
    async fn open_socks5(
        _: &FingerprintConnector,
        _: &str,
        _: u16,
        _: &url::Url,
    ) -> Result<(Self, SocketAddr), String> {
        Err(SOCKS_FEATURE_OFF.to_string())
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

fn carries_udp(proxy_scheme: &str) -> bool {
    matches!(proxy_scheme, "socks5" | "socks5h")
}

const PROXY_NO_UDP: &str = "HTTP/3 needs a socks5:// or socks5h:// proxy with UDP ASSOCIATE; \
                            http and https proxies cannot carry QUIC, use Auto or Http2";

const SOCKS_FEATURE_OFF: &str = "HTTP/3 through a SOCKS5 proxy needs the `socks` feature of \
                                 leyline-http; enable it, or use Auto or Http2";

pub(crate) fn h3_proxy_blocker(proxy: Option<&str>) -> Option<&'static str> {
    let udp = url::Url::parse(proxy?).is_ok_and(|url| carries_udp(url.scheme()));
    match (udp, cfg!(feature = "socks")) {
        (true, true) => None,
        (true, false) => Some(SOCKS_FEATURE_OFF),
        (false, _) => Some(PROXY_NO_UDP),
    }
}

pub(crate) fn proxy_carries_h3(proxy: Option<&str>) -> bool {
    h3_proxy_blocker(proxy).is_none()
}
