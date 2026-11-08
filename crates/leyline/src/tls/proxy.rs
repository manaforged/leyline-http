use tokio::net::TcpStream;

use crate::net::proxy::{self as net_proxy, ProxyScheme, http as http_proxy};
use crate::net::{NetError, ProxyUrl};
use crate::tls::AlpnPlan;
use crate::tls::error::TlsError;
use crate::tls::session_cache::SessionCache;
use crate::tls::{FingerprintConnector, TlsIo, TlsStream};

/// How a request reaches its origin through a proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Route {
    Connect,
    Forward,
}

pub(crate) enum Tunnel {
    Plain(TcpStream),
    Tls(TlsIo),
}

/// Opens a stream through `proxy`: a tunnel to `host:port` for `Route::Connect`, or the proxy connection itself for `Route::Forward`.
pub(crate) async fn open(
    connector: &FingerprintConnector,
    host: &str,
    port: u16,
    proxy: &ProxyUrl,
    route: Route,
) -> Result<Tunnel, TlsError> {
    let url = proxy.url();
    let dialer = connector.dialer();
    match (proxy.scheme(), route) {
        (ProxyScheme::Socks5 | ProxyScheme::Socks5h, _) => Ok(Tunnel::Plain(
            net_proxy::socks(dialer, host, port, proxy).await?,
        )),
        (ProxyScheme::Http, Route::Connect) => Ok(Tunnel::Plain(
            http_proxy::tunnel(dialer, host, port, url).await?,
        )),
        (ProxyScheme::Http, Route::Forward) => Ok(Tunnel::Plain(
            net_proxy::connect_to_proxy(dialer, url).await?,
        )),
        (ProxyScheme::Https, Route::Connect) => tunnel_via_tls(connector, host, port, url)
            .await
            .map(Tunnel::Tls),
        (ProxyScheme::Https, Route::Forward) => open_tls_to_proxy(connector, url)
            .await
            .map(|tls| Tunnel::Tls(tls.stream)),
    }
}

async fn tunnel_via_tls(
    connector: &FingerprintConnector,
    host: &str,
    port: u16,
    proxy: &url::Url,
) -> Result<TlsIo, TlsError> {
    let mut tunnel = open_tls_to_proxy(connector, proxy).await?.stream;
    http_proxy::write_connect_and_validate(&mut tunnel, host, port, proxy).await?;
    Ok(tunnel)
}

async fn open_tls_to_proxy(
    connector: &FingerprintConnector,
    proxy: &url::Url,
) -> Result<TlsStream, TlsError> {
    if connector.has_origin_tls_identity() {
        return Err(NetError::proxy(
            "https:// proxy is not supported together with a client certificate or certificate \
             pins: the origin TLS identity must not be presented to the proxy. Use an http:// \
             CONNECT or socks5:// proxy, or drop the client cert / pins.",
        )
        .into());
    }

    let proxy_host = proxy
        .host_str()
        .ok_or_else(|| NetError::proxy("https proxy has no host"))?;
    let tcp_stream = net_proxy::connect_to_proxy(connector.dialer(), proxy).await?;

    let proxy_key = SessionCache::key(proxy_host, net_proxy::port(proxy), None);
    connector
        .tls_handshake(tcp_stream, proxy_host, &proxy_key, AlpnPlan::Http11)
        .await
        .map_err(TlsError::into_proxy)
}
