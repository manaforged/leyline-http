//! Proxy-tunnel glue — dispatches to HTTP `CONNECT` or SOCKS5.

use crate::tls::connector::FingerprintConnector;
use crate::tls::error::TlsError;
use crate::tls::TlsStream;

pub(crate) mod http;
#[cfg(feature = "socks")]
pub(crate) mod socks5;

/// Dispatch a proxied TLS connection based on the scheme of `proxy_url`.
///
/// `include_alps` matches the direct-path ALPN policy: `true` for h2,
/// `false` for HTTP/1.1-only (WebSocket upgrade). The caller decides
/// which based on the connection it is ultimately establishing.
pub(crate) async fn connect_through_proxy(
    connector: &FingerprintConnector,
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
