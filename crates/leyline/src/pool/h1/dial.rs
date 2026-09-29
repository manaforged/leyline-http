use tokio::net::TcpStream;

use super::{H1Io, H1PooledError};
use crate::tls::FingerprintConnector;
use crate::tls::proxy;

pub(super) async fn dial_plain(
    connector: &FingerprintConnector,
    host: &str,
    port: u16,
    proxy_url: Option<&str>,
) -> Result<Box<dyn H1Io>, H1PooledError> {
    let Some(proxy_url) = proxy_url else {
        let stream = connector
            .with_timeout(connector.dial_tcp(host, port))
            .await?;
        return Ok(Box::new(stream));
    };
    let parsed = url::Url::parse(proxy_url)
        .map_err(|e| H1PooledError::Config(format!("invalid proxy URL: {e}")))?;
    let io: Box<dyn H1Io> = match parsed.scheme() {
        "http" => Box::new(
            connector
                .with_timeout(proxy::connect_to_proxy(connector, &parsed))
                .await?,
        ),
        "https" => Box::new(
            connector
                .with_timeout(proxy::http::open_tls_to_proxy(connector, &parsed))
                .await?
                .stream,
        ),
        "socks5" | "socks5h" => Box::new(socks_tunnel(connector, host, port, &parsed).await?),
        other => {
            return Err(H1PooledError::Config(format!(
                "unsupported proxy scheme `{other}` for http:// URLs"
            )));
        }
    };
    Ok(io)
}

#[cfg(feature = "socks")]
async fn socks_tunnel(
    connector: &FingerprintConnector,
    host: &str,
    port: u16,
    proxy: &url::Url,
) -> Result<TcpStream, H1PooledError> {
    Ok(connector
        .with_timeout(proxy::socks5::tunnel(connector, host, port, proxy))
        .await?)
}

#[cfg(not(feature = "socks"))]
async fn socks_tunnel(
    _connector: &FingerprintConnector,
    _host: &str,
    _port: u16,
    _proxy: &url::Url,
) -> Result<TcpStream, H1PooledError> {
    Err(crate::tls::TlsError::proxy("SOCKS proxy support requires the `socks` feature").into())
}
