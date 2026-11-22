use super::H1Io;
use crate::error::Error;
use crate::h2::H2Config;
use crate::h2::client::H2Client;
use crate::net::{BlockingTcp, ProxyUrl};
use crate::pool::Io;
use crate::pool::Pool;
use crate::pool::connect::{h1_io, remote_addr, start_h2, tls_info};
use crate::pool::types::PoolKey;
use crate::tls::error::TlsError;
use crate::tls::proxy::{self, Route, Tunnel};
use crate::tls::{AlpnPlan, FingerprintConnector};
use crate::wire::{Scheme, TlsFacts, TlsInfo};

pub(super) enum Dialed<W> {
    H1(Box<dyn H1Io>, TlsInfo),
    H2(H2Client, TlsInfo, W),
}

pub(super) async fn open_new<W: Copy + Send>(
    pool: &Pool,
    key: &PoolKey,
    connector: &FingerprintConnector,
    offer: Option<(&H2Config, W)>,
) -> Result<Dialed<W>, Error> {
    let (host, port, proxy) = (key.host.as_str(), key.port, key.proxy.as_ref());
    match key.scheme {
        Scheme::Https => open_tls(pool, key, connector, offer).await,
        Scheme::Http => {
            let (io, addr) = dial_plain(connector, host, port, proxy, pool.io()).await?;
            let facts = TlsFacts {
                remote_addr: addr,
                ..TlsFacts::default()
            };
            Ok(Dialed::H1(io, TlsInfo::from(facts)))
        }
    }
}

async fn open_tls<W: Copy + Send>(
    pool: &Pool,
    key: &PoolKey,
    connector: &FingerprintConnector,
    offer: Option<(&H2Config, W)>,
) -> Result<Dialed<W>, Error> {
    let (host, port, proxy) = (key.host.as_str(), key.port, key.proxy.as_ref());
    let plan = match offer {
        Some(_) => AlpnPlan::Browser,
        None => AlpnPlan::Http11,
    };
    let tls_stream = connector.connect(host, port, proxy, plan).await?;
    let tls = tls_info(&tls_stream);
    if let Some((h2_config, offered)) = offer
        && tls_stream.is_h2()
    {
        let (handle, tls) = start_h2(pool, key.clone(), tls_stream, tls, h2_config.clone()).await?;
        return Ok(Dialed::H2(handle, tls, offered));
    }
    Ok(Dialed::H1(h1_io(pool, proxy, tls_stream)?, tls))
}

pub(super) async fn dial_plain(
    connector: &FingerprintConnector,
    host: &str,
    port: u16,
    proxy: Option<&ProxyUrl>,
    io: Io,
) -> Result<(Box<dyn H1Io>, Option<std::net::SocketAddr>), Error> {
    let Some(proxy) = proxy else {
        #[cfg(unix)]
        if let Some(path) = connector.dialer().unix_socket() {
            let stream = connector
                .with_timeout(connector.dialer().dial_unix(path))
                .await?;
            return Ok((Box::new(stream), None));
        }
        let stream = connector
            .with_timeout(connector.dialer().dial(host, port))
            .await?;
        let addr = stream.peer_addr().ok();
        let io: Box<dyn H1Io> = match io {
            Io::Blocking(wait) => Box::new(BlockingTcp::new(stream, wait)?),
            Io::Async => Box::new(stream),
        };
        return Ok((io, addr));
    };
    let tunnel = connector
        .with_timeout(proxy::open(connector, host, port, proxy, Route::Forward))
        .await
        .map_err(TlsError::into_proxy)?;
    Ok(match tunnel {
        Tunnel::Plain(tcp) => {
            let addr = tcp.peer_addr().ok();
            (Box::new(tcp), addr)
        }
        Tunnel::Tls(io) => {
            let addr = remote_addr(&io);
            (Box::new(io), addr)
        }
    })
}
