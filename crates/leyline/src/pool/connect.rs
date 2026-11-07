use std::future::Future;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::Instant;

use crate::error::{Error, Kind};
use crate::h2::H2Config;
use crate::h2::client::H2Client;
#[cfg(feature = "http3")]
use crate::quic::{H3Client, H3Config, open_fresh_h3};
#[cfg(feature = "http3")]
use crate::tls::trust::Tls;
use crate::tls::{FingerprintConnector, TlsStream};
use crate::trace;
use crate::util::coalesce::Coalesce;
use crate::wire::Scheme;

use super::checkout_live_h2;
use super::config::Io;
use super::h1::H1Io;
use super::liveness::{Probe, confirm};
use super::pool::{OriginFlag, Pool};
use super::types::{Counter, H1Slot, Opened, PoolKey, Proto};
use crate::profile::Carrier;
use crate::wire::{TlsFacts, TlsInfo};

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
        .map_err(|e| e.duplicate())
}

/// An origin that answered an HTTP/2 offer with another protocol.
#[derive(Debug, Clone)]
pub(crate) struct NotH2 {
    negotiated: String,
}

impl NotH2 {
    pub(crate) fn of(alpn: Option<&[u8]>) -> Self {
        let negotiated = alpn.map_or_else(
            || "none".to_string(),
            |p| String::from_utf8_lossy(p).to_string(),
        );
        Self { negotiated }
    }

    pub(crate) fn error(&self) -> Error {
        Error::new(Kind::Http2).with_message(self.to_string())
    }
}

impl std::fmt::Display for NotH2 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "alpn: negotiated {}, expected h2", self.negotiated)
    }
}

#[derive(Clone)]
pub(crate) enum H2Dial {
    Mux(H2Client),
    Http1(NotH2),
}

pub(crate) enum H2Leg {
    Mux(Opened<H2Client>),
    Http1 { not_h2: NotH2, started: Instant },
}

impl Opened<H2Dial> {
    fn into_leg(self) -> H2Leg {
        let Opened {
            conn,
            tls,
            started,
            connect_ms,
            lane,
            slot,
            probe,
        } = self;
        match conn {
            H2Dial::Mux(conn) => H2Leg::Mux(Opened {
                conn,
                tls,
                started,
                connect_ms,
                lane,
                slot,
                probe,
            }),
            H2Dial::Http1(not_h2) => H2Leg::Http1 { not_h2, started },
        }
    }
}

async fn open_fresh_h2(
    pool: Arc<Pool>,
    connector: FingerprintConnector,
    h2_config: H2Config,
    key: PoolKey,
) -> Result<(H2Dial, TlsInfo), Error> {
    let tls_stream = connector
        .connect(
            &key.host,
            key.port,
            key.proxy.as_ref(),
            crate::tls::AlpnPlan::Browser,
        )
        .await
        .map_err(Error::from)?;
    let tls = tls_info(&tls_stream);
    if !tls_stream.is_h2() {
        let not_h2 = NotH2::of(tls_stream.alpn.as_deref());
        park_h1(&pool, key, tls_stream, tls.clone())?;
        return Ok((H2Dial::Http1(not_h2), tls));
    }

    let (handle, tls) = start_h2(&pool, key, tls_stream, tls, h2_config).await?;
    Ok((H2Dial::Mux(handle), tls))
}

/// Starts HTTP/2 on a negotiated stream and pools the connection.
pub(crate) async fn start_h2(
    pool: &Pool,
    key: PoolKey,
    tls_stream: TlsStream,
    tls: TlsInfo,
    h2_config: H2Config,
) -> Result<(H2Client, TlsInfo), Error> {
    let handle = crate::h2::start(tls_stream.stream, h2_config).await?;
    pool.forget(OriginFlag::H1Only, &key.host, key.port, key.proxy.as_ref());
    Ok(pool.install_mux(key, handle, tls))
}

/// Boxes a stream for HTTP/1.1, on blocking IO when the pool uses it and no proxy carries the stream.
pub(crate) fn h1_io(
    pool: &Pool,
    proxy: Option<&crate::net::ProxyUrl>,
    tls_stream: TlsStream,
) -> std::io::Result<Box<dyn H1Io>> {
    let io = match (pool.io(), proxy) {
        (Io::Blocking(wait), None) => tls_stream.stream.blocking(wait)?,
        _ => tls_stream.stream,
    };
    Ok(Box::new(io))
}

pub(crate) fn tls_info(tls_stream: &TlsStream) -> TlsInfo {
    TlsInfo::from(TlsFacts {
        peer_cert_der: tls_stream.peer_cert_der.clone(),
        version: tls_stream.tls_version.clone(),
        cipher: tls_stream.tls_cipher.clone(),
    })
}

fn park_h1(pool: &Pool, key: PoolKey, tls_stream: TlsStream, tls: TlsInfo) -> std::io::Result<()> {
    let io = h1_io(pool, key.proxy.as_ref(), tls_stream)?;
    pool.remember(OriginFlag::H1Only, &key.host, key.port, key.proxy.as_ref());
    pool.park_h1(&key.h1_key(), H1Slot::new(io), tls);
    pool.note(Counter::Installs);
    Ok(())
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
    proxy: Option<&crate::net::ProxyUrl>,
) -> Result<Negotiated, Error> {
    let mut key = pool.key(Scheme::Https, host, port, proxy, Carrier::Tcp);
    Ok(
        match checkout_h2_opened(pool, connector, h2_config, &mut key, Instant::now()).await? {
            None => Negotiated::H1(None),
            Some(H2Leg::Mux(opened)) => Negotiated::H2(opened),
            Some(H2Leg::Http1 { started, .. }) => Negotiated::H1(
                pool.checkout_h1(&key.h1_key(), started)
                    .map(|parked| Opened::fresh(parked, started)),
            ),
        },
    )
}

/// Checks out an HTTP/2 lane, or returns `None` for an origin remembered as HTTP/1.1-only.
pub(super) async fn checkout_h2_opened(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    key: &mut PoolKey,
    now: Instant,
) -> Result<Option<H2Leg>, Error> {
    if pool.recalls(OriginFlag::H1Only, &key.host, key.port, key.proxy.as_ref()) {
        return Ok(None);
    }
    checkout_h2(pool, connector, h2_config, key, now)
        .await
        .map(Some)
}

pub(crate) async fn checkout_h2(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    key: &mut PoolKey,
    now: Instant,
) -> Result<H2Leg, Error> {
    let leg = pick_h2_lane(pool, connector, h2_config, key, now).await;
    let reused = matches!(&leg, Ok(H2Leg::Mux(opened)) if opened.connect_ms.is_none());
    pool.note_checkout(Proto::H2, reused);
    leg
}

async fn pick_h2_lane(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    key: &mut PoolKey,
    now: Instant,
) -> Result<H2Leg, Error> {
    pool.evict_idle(now);
    let lanes = pool.h2_lanes();
    loop {
        let (least, missing) = match scan_lanes(pool, key, now, lanes).await {
            ControlFlow::Break(free) => return Ok(H2Leg::Mux(free)),
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
                return Ok(H2Leg::Mux(opened(hit).on_lane(used)));
            }
        };
        key.lane = lane;
        let started = Instant::now();
        let failed = match open_h2(pool, connector, h2_config, key.clone()).await {
            Ok(H2Leg::Mux(opened)) => {
                if let Some(slot) = opened.conn.reserve() {
                    return Ok(H2Leg::Mux(opened.on_lane(lane).with_slot(Some(slot))));
                }
                continue;
            }
            Ok(H2Leg::Http1 { not_h2, .. }) => Ok(not_h2),
            Err(e) => Err(e),
        };
        let Some((hit, used)) = least else {
            return failed.map(|not_h2| H2Leg::Http1 { not_h2, started });
        };
        match &failed {
            Ok(not_h2) => spill_failed(pool, key, not_h2),
            Err(e) => spill_failed(pool, key, e),
        }
        key.lane = used;
        return Ok(H2Leg::Mux(opened(hit).on_lane(used)));
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

fn spill_failed(pool: &Pool, key: &PoolKey, e: &dyn std::fmt::Display) {
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
) -> Result<H2Leg, Error> {
    let opened = open_coalesced(
        pool.inflight_h2(),
        &key,
        Kind::Http2,
        || {
            pool.checkout_mux::<H2Client>(&key)
                .map(|(conn, tls)| (H2Dial::Mux(conn), tls))
        },
        || {
            open_fresh_h2(
                Arc::clone(pool),
                connector.clone(),
                h2_config.clone(),
                key.clone(),
            )
        },
    )
    .await?;
    Ok(opened.into_leg())
}

/// Returns a pooled or new HTTP/2 connection to the origin, or `None` when it answers with HTTP/1.1.
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
    proxy: Option<&crate::net::ProxyUrl>,
) -> Result<Option<(H2Client, TlsInfo)>, Error> {
    let mut key = pool.key(Scheme::Https, host, port, proxy, Carrier::Tcp);
    loop {
        let mut opened =
            match checkout_h2_opened(pool, connector, h2_config, &mut key, Instant::now()).await? {
                Some(H2Leg::Mux(opened)) => opened,
                None | Some(H2Leg::Http1 { .. }) => return Ok(None),
            };
        let live = match opened.probe.take() {
            Some(probe) => confirm(pool, &key, &opened.conn, probe).await,
            None => true,
        };
        if live {
            return Ok(Some((opened.conn, opened.tls)));
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
    trace::connect(&key.host, key.port, false);
    if trace::on() {
        trace::tls(
            &key.host,
            tls.version.as_deref(),
            tls.cipher.as_deref(),
            Some("h3"),
        );
    }
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
        pool.inflight_h3(),
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
    proxy: Option<&crate::net::ProxyUrl>,
) -> Result<Opened<H3Client>, Error> {
    let key = pool.key(Scheme::Https, host, port, proxy, Carrier::Quic);
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
    pool.note_checkout(Proto::H3, hit.is_some());
    if let Some(hit) = hit {
        return Ok(Opened::pooled(hit));
    }
    open_h3(pool, target, key).await
}

#[cfg(test)]
#[path = "connect_tests.rs"]
mod tests;
