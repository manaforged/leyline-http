#![forbid(unsafe_code)]
use std::time::Duration;

use tokio::net::TcpStream;

use crate::tls::TlsStream;
use crate::tls::error::TlsError;

pub(crate) mod http;
#[cfg(feature = "socks")]
pub(crate) mod socks5;

const PROXY_CONNECT_CEILING: Duration = Duration::from_secs(30);

pub(crate) async fn connect_to_proxy<C: crate::tls::TlsHandshake>(
    connector: &C,
    proxy: &url::Url,
    fallback_port: u16,
) -> Result<TcpStream, TlsError> {
    let host = proxy
        .host_str()
        .ok_or_else(|| TlsError::Profile(format!("{} proxy has no host", proxy.scheme())))?;
    let port = proxy.port_or_known_default().unwrap_or(fallback_port);
    match tokio::time::timeout(PROXY_CONNECT_CEILING, connector.dial_tcp(host, port)).await {
        Ok(res) => res,
        Err(_) => Err(TlsError::TcpConnect(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("proxy connect to {host}:{port} timed out after {PROXY_CONNECT_CEILING:?}"),
        ))),
    }
}

pub(crate) async fn connect_through_proxy<C: crate::tls::TlsHandshake>(
    connector: &C,
    host: &str,
    port: u16,
    proxy_url: &str,
    include_alps: bool,
) -> Result<TlsStream, TlsError> {
    let proxy = url::Url::parse(proxy_url)
        .map_err(|e| TlsError::Profile(format!("invalid proxy URL: {e}")))?;

    match proxy.scheme() {
        "socks5" | "socks5h" => {
            #[cfg(feature = "socks")]
            return socks5::connect(connector, host, port, &proxy, include_alps).await;
            #[cfg(not(feature = "socks"))]
            return Err(TlsError::Profile(
                "SOCKS proxy support requires the `socks` feature".into(),
            ));
        }
        "http" => http::connect(connector, host, port, &proxy, include_alps).await,
        "https" => http::connect_via_tls(connector, host, port, &proxy, include_alps).await,
        other => Err(TlsError::Profile(format!(
            "unsupported proxy scheme `{other}`: leyline tunnels through http://, https://, or \
             socks5:// proxies. Sending CONNECT to a `{other}` proxy would transmit it — including \
             any Proxy-Authorization credentials — in cleartext."
        ))),
    }
}

#[cfg(test)]
mod tests;
