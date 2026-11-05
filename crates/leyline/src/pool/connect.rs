use std::future::Future;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::Instant;

use crate::h2::client::H2Client;
use crate::h2::config::H2Config;
#[cfg(feature = "http3")]
use crate::quic::{H3Client, H3Config, open_fresh_h3};
#[cfg(feature = "http3")]
use crate::tls::Tls;
use crate::tls::{FingerprintConnector, TlsStream};
use crate::trace;
use crate::util::coalesce::Coalesce;
use crate::{Error, Kind};

use super::checkout_live_h2;
use super::liveness::{Probe, confirm};
use super::pool::{OriginFlag, Pool};
use super::types::{H1Slot, Opened, PoolKey, Proto, TlsInfo, Transport};

type Connected<C> = Result<(C, TlsInfo), Arc<Error>>;

pub(crate) type Inflight<C> = Coalesce<PoolKey, Connected<C>>;

async fn open_coalesced<C, Fut>(
    inflight: &Inflight<C>,
    key: &PoolKey,
    failed: Kind,
    checkout: impl Fn() -> Option<(C, TlsInfo)>,
    open: impl Fn() -> Fut,
) -> Result<Opened<C>, Error>
where
    C: Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<(C, TlsInfo), Error>> + Send + 'static,
{
    if let Some(hit) = checkout() {
        return Ok(Opened::pooled(hit));
    }
    let started = Instant::now();
    inflight
        .join_or_spawn(
            key,
            || {
                let opening = open();
                trace::carry(async move { opening.await.map_err(Arc::new) })
            },
            move |e| {
                Err(Arc::new(
                    Error::new(failed).with_message(format!("connect task failed: {e}")),
                ))
            },
        )
        .await
        .map(|opened| Opened::fresh(opened, started))
        .map_err(|e| connect_err(&e))
}

async fn open_fresh_h2(
    pool: Arc<Pool>,
    connector: FingerprintConnector,
    h2_config: H2Config,
    key: PoolKey,
    proxy: Option<crate::core::ProxyUrl>,
) -> Result<(H2Client, TlsInfo), Error> {
    let tls_stream = connector
        .connect(
            &key.host,
            key.port,
            proxy.as_ref().map(crate::core::ProxyUrl::url),
        )
        .await
        .map_err(Error::from)?;
    let tls = tls_info(&tls_stream);
    if tls_stream.alpn.as_deref() != Some(crate::tls::alpn::H2) {
        let negotiated = tls_stream
            .alpn
            .as_ref()
            .map(|p| String::from_utf8_lossy(p).to_string())
            .unwrap_or_else(|| "none".to_string());
        park_h1(&pool, key, tls_stream, tls);
        return Err(Error::new(Kind::Http2)
            .with_message(format!("alpn: negotiated {negotiated}, expected h2"))
            .with_alpn(negotiated));
    }

    let handle = crate::h2::start(tls_stream.stream, h2_config)
        .await
        .map_err(Error::from)?;

    Ok(pool.install_mux(key, handle, tls))
}

pub(crate) fn tls_info(tls_stream: &TlsStream) -> TlsInfo {
    TlsInfo::from(super::types::TlsFacts {
        peer_cert_der: tls_stream.peer_cert_der.clone(),
        version: tls_stream.tls_version.clone(),
        cipher: tls_stream.tls_cipher.clone(),
    })
}

fn park_h1(pool: &Pool, key: PoolKey, tls_stream: TlsStream, tls: TlsInfo) {
    pool.remember(OriginFlag::H1Only, &key.host, key.port, key.proxy.as_ref());
    pool.return_h1(&key, H1Slot::new(Box::new(tls_stream.stream)), tls);
    pool.note_h1_install();
}

pub(crate) enum Negotiated {
    H2(Opened<H2Client>),
    H1(Option<Opened<H1Slot>>),
}

pub(crate) async fn negotiate(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    host: &str,
    port: u16,
    proxy: Option<&crate::core::ProxyUrl>,
) -> Result<Negotiated, Error> {
    if pool.recalls(OriginFlag::H1Only, host, port, proxy) {
        return Ok(Negotiated::H1(None));
    }
    let mut key = pool.key("https", host, port, proxy, Transport::Tcp);
    match checkout_h2_opened(pool, connector, h2_config, &mut key, Instant::now()).await {
        Ok(opened) => Ok(Negotiated::H2(opened)),
        Err((e, started)) if e.alpn().is_some() => Ok(Negotiated::H1(
            pool.checkout_h1(&PoolKey { lane: 0, ..key }, started)
                .map(|parked| Opened::fresh(parked, started)),
        )),
        Err((e, _)) => Err(e),
    }
}

pub(crate) async fn checkout_h2_opened(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    key: &mut PoolKey,
    now: Instant,
) -> Result<Opened<H2Client>, (Error, Instant)> {
    let opened = pick_h2_lane(pool, connector, h2_config, key, now).await;
    let reused = matches!(&opened, Ok(opened) if opened.connect_ms.is_none());
    pool.counters.checkout(Proto::H2, reused);
    opened
}

async fn pick_h2_lane(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    key: &mut PoolKey,
    now: Instant,
) -> Result<Opened<H2Client>, (Error, Instant)> {
    pool.evict_idle(now);
    let lanes = pool.config.h2_lanes();
    loop {
        let (least, missing) = match scan_lanes(pool, key, now, lanes).await {
            ControlFlow::Break(free) => return Ok(free),
            ControlFlow::Continue(scan) => scan,
        };
        let paused = pool.recalls(
            OriginFlag::H2SpillFailed,
            &key.host,
            key.port,
            key.proxy.as_ref(),
        );
        let (least, lane) = match (least, missing) {
            (None, missing) => (None, missing.unwrap_or(0)),
            (Some(full), Some(lane)) if !paused => (Some(full), lane),
            (Some((hit, used)), _) => {
                key.lane = used;
                return Ok(opened(hit).on_lane(used));
            }
        };
        key.lane = lane;
        let started = Instant::now();
        match open_h2(pool, connector, h2_config, key.clone()).await {
            Ok(opened) => {
                if let Some(slot) = opened.conn.reserve() {
                    return Ok(opened.on_lane(lane).with_slot(Some(slot)));
                }
            }
            Err(e) => match least {
                Some((hit, used)) => {
                    spill_failed(pool, key, &e);
                    key.lane = used;
                    return Ok(opened(hit).on_lane(used));
                }
                None => return Err((e, started)),
            },
        }
    }
}

type Pooled = (H2Client, TlsInfo, Option<Probe>);

type Scan = (Option<(Pooled, u8)>, Option<u8>);

async fn scan_lanes(
    pool: &Arc<Pool>,
    key: &mut PoolKey,
    now: Instant,
    lanes: u8,
) -> ControlFlow<Opened<H2Client>, Scan> {
    let mut least: Option<(Pooled, u8)> = None;
    let mut missing = None;
    for lane in 0..lanes {
        key.lane = lane;
        let Some(hit) = checkout_live_h2(pool, key, now).await else {
            missing.get_or_insert(lane);
            if least.is_some() {
                break;
            }
            continue;
        };
        if let Some(slot) = hit.0.reserve() {
            return ControlFlow::Break(opened(hit).on_lane(lane).with_slot(Some(slot)));
        }
        if least
            .as_ref()
            .is_none_or(|(l, _)| hit.0.load() < l.0.load())
        {
            least = Some((hit, lane));
        }
        if missing.is_some() {
            break;
        }
    }
    ControlFlow::Continue((least, missing))
}

fn opened((conn, tls, probe): Pooled) -> Opened<H2Client> {
    Opened::pooled((conn, tls)).with_probe(probe)
}

fn spill_failed(pool: &Pool, key: &PoolKey, e: &Error) {
    tracing::debug!(
        target: "leyline::pool",
        host = %key.host,
        port = key.port,
        error = %e,
        "extra h2 connection failed -- queueing on a full one"
    );
    pool.remember(
        OriginFlag::H2SpillFailed,
        &key.host,
        key.port,
        key.proxy.as_ref(),
    );
}

pub(crate) async fn open_h2(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    key: PoolKey,
) -> Result<Opened<H2Client>, Error> {
    open_coalesced(
        &pool.inflight_h2,
        &key,
        Kind::Http2,
        || pool.checkout_mux::<H2Client>(&key),
        || {
            open_fresh_h2(
                Arc::clone(pool),
                connector.clone(),
                h2_config.clone(),
                key.clone(),
                key.proxy.clone(),
            )
        },
    )
    .await
}

#[tracing::instrument(
    name = "pool.checkout_handle",
    level = "debug",
    skip_all,
    fields(host, port, proxied = proxy.is_some())
)]
pub async fn checkout_handle(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    host: &str,
    port: u16,
    proxy: Option<&crate::core::ProxyUrl>,
) -> Result<(H2Client, TlsInfo), Error> {
    let mut key = pool.key("https", host, port, proxy, Transport::Tcp);
    loop {
        let mut opened = checkout_h2_opened(pool, connector, h2_config, &mut key, Instant::now())
            .await
            .map_err(|(e, _)| e)?;
        let live = match opened.probe.take() {
            Some(probe) => confirm(pool, &key, &opened.conn, probe).await,
            None => true,
        };
        if live {
            return Ok((opened.conn, opened.tls));
        }
    }
}

#[cfg(feature = "http3")]
pub(crate) struct H3Target<'a> {
    pub(crate) config: &'a H3Config,
    pub(crate) trust: &'a Tls,
    pub(crate) connector: &'a FingerprintConnector,
}

#[cfg(feature = "http3")]
async fn open_fresh_h3_installed(
    pool: Arc<Pool>,
    config: H3Config,
    trust: Tls,
    connector: FingerprintConnector,
    key: PoolKey,
) -> Result<(H3Client, TlsInfo), Error> {
    let (handle, tls) = open_fresh_h3(
        &config,
        &trust,
        &connector,
        &key.host,
        key.port,
        key.proxy.as_ref(),
    )
    .await?;
    pool.forget(
        OriginFlag::H3Broken,
        &key.host,
        key.port,
        key.proxy.as_ref(),
    );
    Ok(pool.install_mux(key, handle, tls))
}

#[cfg(feature = "http3")]
pub(crate) async fn open_h3(
    pool: &Arc<Pool>,
    target: &H3Target<'_>,
    key: PoolKey,
) -> Result<Opened<H3Client>, Error> {
    open_coalesced(
        &pool.inflight_h3,
        &key,
        Kind::Http3,
        || pool.checkout_mux::<H3Client>(&key),
        || {
            open_fresh_h3_installed(
                Arc::clone(pool),
                target.config.clone(),
                target.trust.clone(),
                target.connector.clone(),
                key.clone(),
            )
        },
    )
    .await
}

#[cfg(feature = "http3")]
#[tracing::instrument(
    name = "pool.checkout_h3_handle",
    level = "debug",
    skip_all,
    fields(host, port, proxied = proxy.is_some())
)]
pub(crate) async fn checkout_h3_handle(
    pool: &Arc<Pool>,
    target: &H3Target<'_>,
    host: &str,
    port: u16,
    proxy: Option<&crate::core::ProxyUrl>,
) -> Result<Opened<H3Client>, Error> {
    let key = pool.key("https", host, port, proxy, Transport::Quic);
    checkout_h3_opened(pool, target, key).await
}

#[cfg(feature = "http3")]
pub(crate) async fn checkout_h3_opened(
    pool: &Arc<Pool>,
    target: &H3Target<'_>,
    key: PoolKey,
) -> Result<Opened<H3Client>, Error> {
    pool.evict_idle(Instant::now());
    let hit = pool.checkout_mux::<H3Client>(&key);
    pool.counters.checkout(Proto::H3, hit.is_some());
    if let Some(hit) = hit {
        return Ok(Opened::pooled(hit));
    }
    open_h3(pool, target, key).await
}

pub(super) fn connect_err(err: &Error) -> Error {
    let mut out = Error::new(err.cause());
    let mut sourced = true;
    if let Some(tls) = err.tls() {
        out = out.with_source(tls.duplicate());
    } else if let Some(h2) = err.h2() {
        out = out.with_source(h2.duplicate());
    } else if let Some(io) = err.io() {
        out = out.with_source(std::io::Error::new(io.kind(), io.to_string()));
    } else {
        sourced = false;
    }
    if let Some(msg) = err.message() {
        out = out.with_message(msg.to_owned());
    } else if !sourced {
        out = out.with_message(err.to_string());
    }
    if let Some(alpn) = err.alpn() {
        out = out.with_alpn(alpn);
    }
    if let Some(url) = err.url() {
        out = out.with_url(url.clone());
    }
    if let Some(status) = err.status() {
        out = out.with_status(status);
    }
    out
}

#[cfg(test)]
#[path = "connect_tests.rs"]
mod tests;
