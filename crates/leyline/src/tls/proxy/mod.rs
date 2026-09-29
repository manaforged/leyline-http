#![forbid(unsafe_code)]
use tokio::net::TcpStream;

use crate::tls::TlsStream;
use crate::tls::error::TlsError;

pub(crate) mod http;
#[cfg(feature = "socks")]
pub(crate) mod socks5;

pub(crate) async fn connect_to_proxy<C: crate::tls::TlsHandshake>(
    connector: &C,
    proxy: &url::Url,
    fallback_port: u16,
) -> Result<TcpStream, TlsError> {
    let host = proxy
        .host_str()
        .ok_or_else(|| TlsError::proxy(format!("{} proxy has no host", proxy.scheme())))?;
    let port = proxy.port_or_known_default().unwrap_or(fallback_port);
    connector
        .dial_tcp(host, port)
        .await
        .map_err(TlsError::into_proxy)
}

pub(crate) async fn connect_through_proxy<C: crate::tls::TlsHandshake>(
    connector: &C,
    host: &str,
    port: u16,
    proxy_url: &str,
    include_alps: bool,
) -> Result<TlsStream, TlsError> {
    let proxy = url::Url::parse(proxy_url)
        .map_err(|e| TlsError::proxy(format!("invalid proxy URL: {e}")))?;

    match proxy.scheme() {
        "socks5" | "socks5h" => {
            #[cfg(feature = "socks")]
            return socks5::connect(connector, host, port, &proxy, include_alps).await;
            #[cfg(not(feature = "socks"))]
            return Err(TlsError::proxy(
                "SOCKS proxy support requires the `socks` feature",
            ));
        }
        "http" => http::connect(connector, host, port, &proxy, include_alps).await,
        "https" => http::connect_via_tls(connector, host, port, &proxy, include_alps).await,
        other => Err(TlsError::proxy(format!(
            "unsupported proxy scheme `{other}`: use an http, https, socks5, or socks5h proxy URL"
        ))),
    }
}

#[cfg(test)]
mod tests;
