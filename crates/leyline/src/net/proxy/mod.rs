#![forbid(unsafe_code)]
use tokio::net::TcpStream;

use super::{Dialer, NetError};

pub(crate) mod http;
mod scheme;
#[cfg(feature = "socks")]
pub(crate) mod socks5;

pub(crate) use self::scheme::ProxyScheme;
pub use self::scheme::ProxyUrl;
#[cfg(feature = "http3")]
pub(crate) use self::scheme::SOCKS_FEATURE_OFF;
#[cfg(not(feature = "socks"))]
pub(crate) use self::scheme::socks_disabled;

const SOCKS5_DEFAULT_PORT: u16 = 1080;

pub(crate) async fn connect_to_proxy(
    dialer: &Dialer,
    proxy: &url::Url,
) -> Result<TcpStream, NetError> {
    let host = proxy
        .host_str()
        .ok_or_else(|| NetError::proxy(format!("{} proxy has no host", proxy.scheme())))?;
    dialer
        .dial(host, port(proxy))
        .await
        .map_err(NetError::into_proxy)
}

pub(crate) fn port(proxy: &url::Url) -> u16 {
    proxy.port_or_known_default().unwrap_or(SOCKS5_DEFAULT_PORT)
}

#[cfg(feature = "socks")]
pub(crate) async fn socks(
    dialer: &Dialer,
    host: &str,
    port: u16,
    proxy: &ProxyUrl,
) -> Result<TcpStream, NetError> {
    socks5::tunnel(dialer, host, port, proxy.url()).await
}

#[cfg(not(feature = "socks"))]
pub(crate) async fn socks(
    _dialer: &Dialer,
    _host: &str,
    _port: u16,
    proxy: &ProxyUrl,
) -> Result<TcpStream, NetError> {
    Err(NetError::proxy(socks_disabled(proxy.as_str())))
}

#[cfg(test)]
mod tests;
