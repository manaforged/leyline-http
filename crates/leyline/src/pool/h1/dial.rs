use tokio::net::TcpStream;

use super::H1Io;
use crate::Error;
use crate::h2::client::H2Client;
use crate::h2::config::H2Config;
use crate::pool::connect::tls_info;
use crate::pool::types::PoolKey;
use crate::pool::{Pool, TlsInfo};
use crate::tls::proxy;
use crate::tls::{FingerprintConnector, TlsError, TlsStream};

#[derive(Clone, Copy)]
pub(crate) enum H1Dial<'a> {
    Browser(&'a H2Config),
    Http1Only,
}

pub(super) enum Dialed {
    H1(Box<dyn H1Io>, TlsInfo),
    H2(H2Client, TlsInfo),
}

pub(super) async fn open_new(
    pool: &Pool,
    key: &PoolKey,
    connector: &FingerprintConnector,
    dial: H1Dial<'_>,
) -> Result<Dialed, Error> {
    let (host, port, proxy) = (key.host.as_str(), key.port, key.proxy.as_ref());
    match key.scheme.as_ref() {
        "https" => open_tls(pool, key, connector, dial).await,
        "http" => Ok(Dialed::H1(
            dial_plain(connector, host, port, proxy, pool.config.blocking).await?,
            TlsInfo::default(),
        )),
        other => Err(Error::config(format!(
            "unsupported URL scheme for HTTP/1.1: {other}"
        ))),
    }
}

async fn open_tls(
    pool: &Pool,
    key: &PoolKey,
    connector: &FingerprintConnector,
    dial: H1Dial<'_>,
) -> Result<Dialed, Error> {
    let (host, port, proxy) = (key.host.as_str(), key.port, key.proxy.as_ref());
    let tls_stream = match dial {
        H1Dial::Browser(_) => {
            connector
                .connect(host, port, proxy.map(crate::core::ProxyUrl::url))
                .await?
        }
        H1Dial::Http1Only => {
            connector
                .connect_h1(host, port, proxy.map(crate::core::ProxyUrl::url))
                .await?
        }
    };
    let tls = tls_info(&tls_stream);
    if let H1Dial::Browser(h2_config) = dial
        && tls_stream.alpn.as_deref() == Some(crate::tls::alpn::H2)
    {
        return hand_to_h2(pool, key, tls_stream, tls, h2_config).await;
    }
    let io = match (pool.config.blocking, proxy) {
        (Some(wait), None) => tls_stream.stream.blocking(wait)?,
        _ => tls_stream.stream,
    };
    Ok(Dialed::H1(Box::new(io), tls))
}

async fn hand_to_h2(
    pool: &Pool,
    key: &PoolKey,
    tls_stream: TlsStream,
    tls: TlsInfo,
    h2_config: &H2Config,
) -> Result<Dialed, Error> {
    let handle = crate::h2::start(tls_stream.stream, h2_config.clone()).await?;
    let (handle, tls) = pool.install_mux(key.clone(), handle, tls);
    Ok(Dialed::H2(handle, tls))
}

pub(super) async fn dial_plain(
    connector: &FingerprintConnector,
    host: &str,
    port: u16,
    proxy: Option<&crate::core::ProxyUrl>,
    blocking: Option<Option<std::time::Duration>>,
) -> Result<Box<dyn H1Io>, Error> {
    let Some(proxy) = proxy else {
        let stream = connector
            .with_timeout(connector.dial_tcp(host, port))
            .await?;
        return Ok(match blocking {
            Some(wait) => Box::new(crate::tls::BlockingTcp::new(stream, wait)?),
            None => Box::new(stream),
        });
    };
    let parsed = proxy.url();
    let io: Box<dyn H1Io> = match parsed.scheme() {
        "http" => Box::new(
            connector
                .with_timeout(proxy::connect_to_proxy(connector, parsed))
                .await
                .map_err(TlsError::into_proxy)?,
        ),
        "https" => Box::new(
            connector
                .with_timeout(proxy::http::open_tls_to_proxy(connector, parsed))
                .await
                .map_err(TlsError::into_proxy)?
                .stream,
        ),
        "socks5" | "socks5h" => Box::new(socks_tunnel(connector, host, port, parsed).await?),
        other => {
            return Err(Error::config(format!(
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
) -> Result<TcpStream, Error> {
    Ok(connector
        .with_timeout(proxy::socks5::tunnel(connector, host, port, proxy))
        .await
        .map_err(TlsError::into_proxy)?)
}

#[cfg(not(feature = "socks"))]
async fn socks_tunnel(
    _connector: &FingerprintConnector,
    _host: &str,
    _port: u16,
    _proxy: &url::Url,
) -> Result<TcpStream, Error> {
    Err(TlsError::proxy("SOCKS proxy support requires the `socks` feature").into())
}
