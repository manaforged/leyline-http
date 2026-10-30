#![forbid(unsafe_code)]
use tokio::net::TcpStream;

use crate::tls::alpn::AlpnPlan;
use crate::tls::error::TlsError;
use crate::tls::{SessionCache, TlsIo, TlsStream};

pub(crate) mod http;
#[cfg(feature = "socks")]
pub(crate) mod socks5;

const SOCKS5_DEFAULT_PORT: u16 = 1080;

pub(crate) async fn connect_to_proxy(
    connector: &crate::tls::FingerprintConnector,
    proxy: &url::Url,
) -> Result<TcpStream, TlsError> {
    let host = proxy
        .host_str()
        .ok_or_else(|| TlsError::proxy(format!("{} proxy has no host", proxy.scheme())))?;
    connector
        .dial_tcp(host, port(proxy))
        .await
        .map_err(TlsError::into_proxy)
}

fn port(proxy: &url::Url) -> u16 {
    proxy.port_or_known_default().unwrap_or(SOCKS5_DEFAULT_PORT)
}

pub(crate) enum Tunnel {
    Plain(TcpStream),
    Tls(TlsIo),
}

pub(crate) fn parse(proxy_url: &str) -> Result<url::Url, TlsError> {
    url::Url::parse(proxy_url).map_err(|e| TlsError::proxy(format!("invalid proxy URL: {e}")))
}

pub(crate) async fn open_tunnel(
    connector: &crate::tls::FingerprintConnector,
    host: &str,
    port: u16,
    proxy: &url::Url,
) -> Result<Tunnel, TlsError> {
    match proxy.scheme() {
        "socks5" | "socks5h" => {
            #[cfg(feature = "socks")]
            return socks5::tunnel(connector, host, port, proxy)
                .await
                .map(Tunnel::Plain);
            #[cfg(not(feature = "socks"))]
            return Err(TlsError::proxy(
                "SOCKS proxy support requires the `socks` feature",
            ));
        }
        "http" => http::tunnel(connector, host, port, proxy)
            .await
            .map(Tunnel::Plain),
        "https" => http::tunnel_via_tls(connector, host, port, proxy)
            .await
            .map(Tunnel::Tls),
        other => Err(TlsError::proxy(format!(
            "unsupported proxy scheme `{other}`: use an http, https, socks5, or socks5h proxy URL"
        ))),
    }
}

pub(crate) async fn handshake(
    connector: &crate::tls::FingerprintConnector,
    tunnel: Tunnel,
    host: &str,
    port: u16,
    proxy: &url::Url,
    plan: AlpnPlan,
) -> Result<TlsStream, TlsError> {
    let session_key = SessionCache::key(host, port, Some(proxy));
    match tunnel {
        Tunnel::Plain(tcp) => connector.tls_handshake(tcp, host, &session_key, plan).await,
        Tunnel::Tls(io) => {
            connector
                .tls_handshake_nested(io, host, &session_key, plan)
                .await
        }
    }
}

#[cfg(test)]
mod tests;
