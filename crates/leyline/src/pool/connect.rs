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
use crate::tls::TlsTrustConfig;
use crate::tls::{FingerprintConnector, TlsStream};
use crate::trace;
use crate::util::lock;
use crate::{Error, Kind};

use super::checkout_live_h2;
use super::pool::Pool;
use super::types::{H1Slot, Opened, PoolKey, TlsInfo, Transport};

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
        failed: Kind,
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
    failed: Kind,
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
    if tls_stream.alpn.as_deref() != Some(b"h2") {
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

    Ok(pool.install_h2(key, handle, tls))
}

pub(crate) fn tls_info(tls_stream: &TlsStream) -> TlsInfo {
    TlsInfo {
        peer_cert_der: tls_stream.peer_cert_der.clone(),
        version: tls_stream.tls_version.clone(),
        cipher: tls_stream.tls_cipher.clone(),
    }
}

fn park_h1(pool: &Pool, key: PoolKey, tls_stream: TlsStream, tls: TlsInfo) {
    pool.note_h1_only(&key.host, key.port, key.proxy.as_deref());
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
    if pool.is_h1_only(host, port, proxy) {
        return Ok(Negotiated::H1(None));
    }
    let key = pool.key("https", host, port, proxy, Transport::Tcp);
    pool.evict_idle();
    if let Some(hit) = checkout_live_h2(pool, &key).await {
        return Ok(Negotiated::H2(Opened::pooled(hit)));
    }
    let started = Instant::now();
    match open_h2(pool, connector, h2_config, key.clone()).await {
        Ok(opened) => Ok(Negotiated::H2(Opened::fresh(opened, started))),
        Err(e) if e.alpn().is_some() => Ok(Negotiated::H1(
            pool.checkout_h1(&key)
                .map(|parked| Opened::fresh(parked, started)),
        )),
        Err(e) => Err(e),
    }
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
        Kind::Http2,
        || pool.checkout_h2(&key),
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

    pool.evict_idle();

    if let Some((handle, tls)) = checkout_live_h2(pool, &key).await {
        return Ok((handle, tls));
    }

    open_h2(pool, connector, h2_config, key).await
}

#[cfg(feature = "http3")]
pub(crate) struct H3Target<'a> {
    pub(crate) config: &'a H3Config,
    pub(crate) trust: &'a TlsTrustConfig,
    pub(crate) connector: &'a FingerprintConnector,
}

#[cfg(feature = "http3")]
async fn open_fresh_h3_installed(
    pool: Arc<Pool>,
    config: H3Config,
    trust: TlsTrustConfig,
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
    Ok(pool.install_or_get_h3(key, handle, tls))
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
        Kind::Http3,
        || pool.checkout_h3(&key),
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
pub async fn checkout_h3_handle(
    pool: &Arc<Pool>,
    target: &H3Target<'_>,
    host: &str,
    port: u16,
    proxy: Option<&str>,
) -> Result<(H3Client, TlsInfo), Error> {
    let key = pool.key("https", host, port, proxy, Transport::Quic);

    pool.evict_idle();

    if let Some(hit) = pool.checkout_h3(&key) {
        return Ok(hit);
    }

    open_h3(pool, target, key).await
}

pub(super) fn connect_err(err: &Error) -> Error {
    let mut out = Error::new(err.kind());
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
