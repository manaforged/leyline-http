use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};

use crate::h2::client::H2Client;
use crate::h2::config::H2Config;
#[cfg(feature = "http3")]
use crate::quic::{H3Client, H3Config, open_fresh_h3};
#[cfg(feature = "http3")]
use crate::tls::Tls;
use crate::tls::{FingerprintConnector, TlsStream};
use crate::trace;
use crate::util::lock;
use crate::{Cause, Error};

use super::checkout_live_h2;
use super::pool::{OriginFlag, Pool};
use super::types::{H1Slot, Opened, PoolKey, Proto, TlsInfo, Transport};

type SharedConnect<C> = Shared<BoxFuture<'static, Result<(C, TlsInfo), Arc<Error>>>>;

type Registry<C> = Arc<Mutex<HashMap<PoolKey, SharedConnect<C>>>>;

pub(crate) struct Inflight<C>(Registry<C>);

impl<C> Clone for Inflight<C> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<C> Default for Inflight<C> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(HashMap::new())))
    }
}

impl<C: Clone + Send + Sync + 'static> Inflight<C> {
    fn join_or_spawn<Fut>(
        &self,
        key: &PoolKey,
        failed: Cause,
        open: impl FnOnce() -> Fut,
    ) -> SharedConnect<C>
    where
        Fut: Future<Output = Result<(C, TlsInfo), Error>> + Send + 'static,
    {
        let mut map = lock(&self.0);
        if let Some(existing) = map.get(key) {
            return existing.clone();
        }
        let registry = Arc::clone(&self.0);
        let cleanup_key = key.clone();
        let open = open();
        let handle = tokio::spawn(trace::carry(async move {
            let result = open.await.map_err(Arc::new);
            lock(&registry).remove(&cleanup_key);
            result
        }));
        let shared = async move {
            handle.await.unwrap_or_else(|e| {
                Err(Arc::new(
                    Error::new(failed).with_message(format!("connect task failed: {e}")),
                ))
            })
        }
        .boxed()
        .shared();
        map.insert(key.clone(), shared.clone());
        shared
    }
}

async fn open_coalesced<C, Fut>(
    inflight: &Inflight<C>,
    key: &PoolKey,
    failed: Cause,
    checkout: impl Fn() -> Option<(C, TlsInfo)>,
    open: impl Fn() -> Fut,
) -> Result<(C, TlsInfo), Error>
where
    C: Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<(C, TlsInfo), Error>> + Send + 'static,
{
    if let Some(hit) = checkout() {
        return Ok(hit);
    }
    inflight
        .join_or_spawn(key, failed, &open)
        .await
        .map_err(|e| connect_err(&e))
}

async fn open_fresh_h2(
    pool: Arc<Pool>,
    connector: FingerprintConnector,
    h2_config: H2Config,
    key: PoolKey,
    proxy: Option<String>,
) -> Result<(H2Client, TlsInfo), Error> {
    let tls_stream = connector
        .connect(&key.host, key.port, proxy.as_deref())
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
        return Err(Error::new(Cause::Http2)
            .with_message(format!("alpn: negotiated {negotiated}, expected h2"))
            .with_alpn(negotiated));
    }

    let handle = crate::h2::start(tls_stream.stream, h2_config)
        .await
        .map_err(Error::from)?;

    Ok(pool.install_mux(key, handle, tls))
}

pub(crate) fn tls_info(tls_stream: &TlsStream) -> TlsInfo {
    TlsInfo {
        peer_cert_der: tls_stream.peer_cert_der.clone(),
        version: tls_stream.tls_version.clone(),
        cipher: tls_stream.tls_cipher.clone(),
    }
}

fn park_h1(pool: &Pool, key: PoolKey, tls_stream: TlsStream, tls: TlsInfo) {
    pool.remember(
        OriginFlag::H1Only,
        &key.host,
        key.port,
        key.proxy.as_deref(),
    );
    pool.return_h1(
        key,
        H1Slot {
            io: Box::new(tls_stream.stream),
        },
        tls,
    );
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
    proxy: Option<&str>,
) -> Result<Negotiated, Error> {
    if pool.recalls(OriginFlag::H1Only, host, port, proxy) {
        return Ok(Negotiated::H1(None));
    }
    let key = pool.key("https", host, port, proxy, Transport::Tcp);
    match checkout_h2_opened(pool, connector, h2_config, key.clone()).await {
        Ok(opened) => Ok(Negotiated::H2(opened)),
        Err((e, started)) if e.alpn().is_some() => Ok(Negotiated::H1(
            pool.checkout_h1(&key)
                .map(|parked| Opened::fresh(parked, started)),
        )),
        Err((e, _)) => Err(e),
    }
}

pub(crate) async fn checkout_h2_opened(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    key: PoolKey,
) -> Result<Opened<H2Client>, (Error, Instant)> {
    let opened = pick_h2_lane(pool, connector, h2_config, key).await;
    let reused = matches!(&opened, Ok(opened) if opened.connect_ms.is_none());
    pool.counters.checkout(Proto::H2, reused);
    opened
}

async fn pick_h2_lane(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    key: PoolKey,
) -> Result<Opened<H2Client>, (Error, Instant)> {
    pool.evict_idle();
    let lanes = pool.config.h2_lanes();
    let mut key = key;
    loop {
        let mut least: Option<((H2Client, TlsInfo), u8)> = None;
        let mut missing = None;
        for lane in 0..lanes {
            key.lane = lane;
            match checkout_live_h2(pool, &key).await {
                Some(hit) => {
                    if let Some(slot) = hit.0.reserve() {
                        return Ok(Opened::pooled(hit).on_lane(lane).with_slot(Some(slot)));
                    }
                    if least
                        .as_ref()
                        .is_none_or(|(l, _)| hit.0.load() < l.0.load())
                    {
                        least = Some((hit, lane));
                    }
                }
                None => {
                    missing.get_or_insert(lane);
                }
            }
            if missing.is_some() && least.is_some() {
                break;
            }
        }
        let paused = pool.recalls(
            OriginFlag::H2SpillFailed,
            &key.host,
            key.port,
            key.proxy.as_deref(),
        );
        let (least, lane) = match (least, missing) {
            (None, missing) => (None, missing.unwrap_or(0)),
            (Some(full), Some(lane)) if !paused => (Some(full), lane),
            (Some((hit, used)), _) => return Ok(Opened::pooled(hit).on_lane(used)),
        };
        key.lane = lane;
        let started = Instant::now();
        match open_h2(pool, connector, h2_config, key.clone()).await {
            Ok(opened) => {
                if let Some(slot) = opened.0.reserve() {
                    return Ok(Opened::fresh(opened, started)
                        .on_lane(lane)
                        .with_slot(Some(slot)));
                }
            }
            Err(e) => match least {
                Some((hit, used)) => {
                    spill_failed(pool, &key, &e);
                    return Ok(Opened::pooled(hit).on_lane(used));
                }
                None => return Err((e, started)),
            },
        }
    }
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
        key.proxy.as_deref(),
    );
}

pub(crate) async fn open_h2(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    key: PoolKey,
) -> Result<(H2Client, TlsInfo), Error> {
    open_coalesced(
        &pool.inflight_h2,
        &key,
        Cause::Http2,
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
    proxy: Option<&str>,
) -> Result<(H2Client, TlsInfo), Error> {
    let key = pool.key("https", host, port, proxy, Transport::Tcp);
    match checkout_h2_opened(pool, connector, h2_config, key).await {
        Ok(opened) => Ok((opened.conn, opened.tls)),
        Err((e, _)) => Err(e),
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
        key.proxy.as_deref(),
    )
    .await?;
    pool.forget(
        OriginFlag::H3Broken,
        &key.host,
        key.port,
        key.proxy.as_deref(),
    );
    Ok(pool.install_mux(key, handle, tls))
}

#[cfg(feature = "http3")]
pub(crate) async fn open_h3(
    pool: &Arc<Pool>,
    target: &H3Target<'_>,
    key: PoolKey,
) -> Result<(H3Client, TlsInfo), Error> {
    open_coalesced(
        &pool.inflight_h3,
        &key,
        Cause::Http3,
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
    proxy: Option<&str>,
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
    pool.evict_idle();
    let hit = pool.checkout_mux::<H3Client>(&key);
    pool.counters.checkout(Proto::H3, hit.is_some());
    if let Some(hit) = hit {
        return Ok(Opened::pooled(hit));
    }
    let started = Instant::now();
    let opened = open_h3(pool, target, key).await?;
    Ok(Opened::fresh(opened, started))
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
