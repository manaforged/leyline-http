//! Proxy-tunnel glue — dispatches to HTTP `CONNECT` or SOCKS5.

use std::time::Duration;

use tokio::net::TcpStream;

use crate::tls::error::TlsError;
use crate::tls::TlsStream;

pub(crate) mod http;
#[cfg(feature = "socks")]
pub(crate) mod socks5;

/// Ceiling on the client→proxy TCP connect. The connector's
/// `connect_timeout` (when the caller sets one) wraps the entire
/// tunnel + TLS phase; this backstop guards callers that never set
/// it, so a hung proxy cannot park the future forever.
const PROXY_CONNECT_CEILING: Duration = Duration::from_secs(30);

/// Open the TCP leg to the proxy itself — shared by the HTTP CONNECT
/// and SOCKS5 tunnels.
///
/// `Url::port()` returns `None` when the port equals the scheme's
/// default (80 for http), so `http://host:80` would silently fall
/// through to the scheme fallback; `port_or_known_default()` preserves
/// explicit default ports (for example a proxy on
/// `proxy.example.com:80`). `fallback_port` covers schemes the `url` crate has no
/// default for (socks5 → 1080, http-proxy convention → 8080).
pub(crate) async fn connect_to_proxy(
    proxy: &url::Url,
    fallback_port: u16,
) -> Result<TcpStream, TlsError> {
    let host = proxy
        .host_str()
        .ok_or_else(|| TlsError::Profile(format!("{} proxy has no host", proxy.scheme())))?;
    let port = proxy.port_or_known_default().unwrap_or(fallback_port);
    let addr = format!("{host}:{port}");
    match tokio::time::timeout(PROXY_CONNECT_CEILING, TcpStream::connect(&addr)).await {
        Ok(res) => res.map_err(TlsError::TcpConnect),
        Err(_) => Err(TlsError::TcpConnect(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("proxy connect to {addr} timed out after {PROXY_CONNECT_CEILING:?}"),
        ))),
    }
}

/// Dispatch a proxied TLS connection based on the scheme of `proxy_url`.
///
/// `include_alps` matches the direct-path ALPN policy: `true` for h2,
/// `false` for HTTP/1.1-only (WebSocket upgrade). The caller decides
/// which based on the connection it is ultimately establishing.
pub(crate) async fn connect_through_proxy<C: crate::tls::TlsHandshake>(
    connector: &C,
    host: &str,
    port: u16,
    proxy_url: &str,
    include_alps: bool,
) -> Result<TlsStream, TlsError> {
    let proxy = url::Url::parse(proxy_url)
        .map_err(|e| TlsError::Profile(format!("invalid proxy URL: {e}")))?;

    if proxy.scheme() == "socks5" || proxy.scheme() == "socks5h" {
        #[cfg(feature = "socks")]
        return socks5::connect(connector, host, port, &proxy, include_alps).await;
        #[cfg(not(feature = "socks"))]
        return Err(TlsError::Profile(
            "SOCKS proxy support requires the `socks` feature".into(),
        ));
    }

    http::connect(connector, host, port, &proxy, include_alps).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connect_to_proxy_reaches_local_listener() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url: url::Url = format!("http://127.0.0.1:{}", addr.port()).parse().unwrap();
        let stream = connect_to_proxy(&url, 8080).await.expect("local connect");
        assert_eq!(stream.peer_addr().unwrap().port(), addr.port());
    }

    #[tokio::test]
    async fn connect_to_proxy_requires_host() {
        // Cannot-be-a-base URLs parse with no host component.
        let url: url::Url = "mailto:a@b".parse().unwrap();
        assert!(connect_to_proxy(&url, 8080).await.is_err());
    }
}
