//! HTTP connection pool.
//!
//! Keys entries by `(host, port, proxy)` and supports two protocol
//! flavours:
//!
//! - **HTTP/2** — one connection per destination; the pooled handle
//!   is [`crate::h2::H2Client`], which is cloneable and multiplexes concurrent
//!   requests over the same TCP connection. Checkout is a cheap
//!   `clone` and many in-flight requests share the connection
//!   without serialisation.
//! - **HTTP/1.1 keep-alive** — a pool of warm connections per destination.
//!   H1 cannot multiplex, so per-host concurrency needs several connections;
//!   the live count is capped at `max_h1_conns_per_host` (default 256,
//!   throughput-favouring; set 6 to mirror a browser's per-host socket limit)
//!   by a per-host semaphore. A checkout takes one warm connection out of the
//!   deque for a request/response exchange and returns it on a reusable
//!   completion; requests beyond the cap wait on the semaphore for a
//!   connection to free, rather than opening unbounded sockets.
//!
//! ## Eviction
//! - **Idle timeout**: entries untouched for longer than the configured
//!   window (default 300 s) are dropped.
//! - **LRU cap**: the pool carries at most `max_connections` entries
//!   (default 2048). H1 and H2 share the cap — no per-protocol limit.
//!   When inserting a new entry would exceed the cap, the
//!   least-recently-used entry is evicted. Dropping an H2 entry's
//!   `DriverTask` triggers a graceful GOAWAY; dropping an H1 entry's
//!   stream closes the TCP connection.
//!
//! The last-use timestamp is updated on every checkout, so LRU
//! ordering reflects actual request activity, not install time.

use std::sync::Arc;

use crate::h2::client::{H2ResponseEx, RequestBody};
use crate::h2::config::H2Config;
use crate::h2::connection::{ClientConnection, PseudoHeaders};
use crate::tls::ConnectorVariant;

mod h1;
#[allow(clippy::module_inception)]
mod pool;
mod types;

pub use h1::{
    send_request_h1_pooled, H1Body, H1Io, H1PooledError, H1Response, H1ResponseBody, H1Target,
    MAX_H1_BODY_BYTES, MAX_H1_HEADER_BYTES,
};
pub use pool::{
    Pool, DEFAULT_IDLE_TIMEOUT, DEFAULT_MAX_CONNECTIONS, DEFAULT_MAX_H1_CONNS_PER_HOST,
};
pub use types::{H1Slot, PoolStats, TlsInfo};

use types::{H2Io, PoolKey};

/// Construct a pool key. Visible to the `h1` submodule.
pub(crate) fn make_key(host: &str, port: u16, proxy: Option<&str>) -> PoolKey {
    PoolKey {
        host: host.to_string(),
        port,
        proxy: proxy.map(|s| s.to_string()),
    }
}

/// Establish a fresh TLS + H2 connection to `(host, port, proxy)`, require
/// that ALPN negotiated `h2`, install the driver into `pool` under `key`,
/// and return the cloneable client handle plus its TLS metadata.
async fn open_fresh_h2(
    pool: &Arc<Pool>,
    connector: &ConnectorVariant,
    h2_config: &H2Config,
    key: PoolKey,
    host: &str,
    port: u16,
    proxy: Option<&str>,
) -> Result<(crate::h2::client::H2Client, TlsInfo), crate::Error> {
    let tls_stream = connector
        .connect(host, port, proxy)
        .await
        .map_err(crate::Error::from)?;
    if tls_stream.alpn.as_deref() != Some(b"h2") {
        let negotiated = tls_stream
            .alpn
            .as_ref()
            .map(|p| String::from_utf8_lossy(p).to_string())
            .unwrap_or_else(|| "none".to_string());
        return Err(crate::Error::AlpnMismatch { negotiated });
    }

    let tls = TlsInfo {
        peer_cert_der: tls_stream.peer_cert_der.clone(),
        version: tls_stream.tls_version.clone(),
        cipher: tls_stream.tls_cipher.clone(),
    };
    let (handle, driver) = ClientConnection::<H2Io>::start(tls_stream.stream, h2_config.clone())
        .await
        .map_err(crate::Error::from)?;

    pool.install_h2(key, handle.clone(), driver, tls.clone());
    Ok((handle, tls))
}

/// Establish — or join an already in-progress — H2 connection to `(host, port, proxy)`,
/// single-flighting concurrent first-requests so they share ONE TLS+H2 handshake instead of each
/// opening their own. This eliminates the connection storm a one-conn-per-host pool otherwise
/// triggers when N requests hit a cold destination at once, and (because the connect runs inside a
/// boxed `Shared` future) keeps the large handshake state machine off every caller's per-request
/// future. H2-only: H1 keeps opening parallel connections via its own path.
///
/// The connect future removes its own in-flight entry on completion, so a cancelled leader cannot
/// strand waiters and the next miss after a connection dies starts a fresh connect.
async fn open_h2_coalesced(
    pool: &Arc<Pool>,
    connector: &ConnectorVariant,
    h2_config: &H2Config,
    key: PoolKey,
    host: &str,
    port: u16,
    proxy: Option<&str>,
) -> Result<(crate::h2::client::H2Client, TlsInfo), crate::Error> {
    use futures_util::FutureExt;

    // A connect may have finished between the caller's pool miss and now.
    if let Some(hit) = pool.checkout_h2(&key) {
        return Ok(hit);
    }

    let shared = pool.inflight_h2_get_or_insert_with(key.clone(), || {
        let pool = Arc::clone(pool);
        let cleanup_pool = Arc::clone(&pool);
        let connector = connector.clone();
        let h2_config = h2_config.clone();
        let connect_key = key.clone();
        let cleanup_key = key.clone();
        let host = host.to_string();
        let proxy = proxy.map(|s| s.to_string());
        async move {
            let result = open_fresh_h2(
                &pool,
                &connector,
                &h2_config,
                connect_key,
                &host,
                port,
                proxy.as_deref(),
            )
            .await
            .map_err(Arc::new);
            // On completion (success or failure, driven by whichever task), drop our in-flight
            // entry. On success the connection is already installed in the pool by
            // `open_fresh_h2`, so subsequent callers checkout-hit rather than re-connect.
            cleanup_pool.inflight_h2_remove(&cleanup_key);
            result
        }
        .boxed()
        .shared()
    });

    match shared.await {
        Ok(pair) => Ok(pair),
        Err(_shared_err) => {
            // The shared connect failed; its error is `Arc`-shared (one failure
            // fanned out to every waiter). Rather than surface that already-stale,
            // shared failure we make our own single fresh attempt — concurrent
            // failures degrade to sequential retries, never a permanent storm.
            Box::pin(open_fresh_h2(
                pool, connector, h2_config, key, host, port, proxy,
            ))
            .await
        }
    }
}

/// Obtain a cloneable [`crate::h2::H2Client`] handle for `(host, port, proxy)`,
/// reusing an existing pooled connection when available and otherwise
/// establishing a fresh TLS + H2 handshake.
#[tracing::instrument(
    name = "pool.checkout_handle",
    level = "debug",
    skip_all,
    fields(host, port, proxied = proxy.is_some())
)]
pub async fn checkout_handle(
    pool: &Arc<Pool>,
    connector: &ConnectorVariant,
    h2_config: &H2Config,
    host: &str,
    port: u16,
    proxy: Option<&str>,
) -> Result<(crate::h2::client::H2Client, TlsInfo), crate::Error> {
    let key = make_key(host, port, proxy);

    pool.evict_idle();

    if let Some((handle, tls)) = pool.checkout_h2(&key) {
        return Ok((handle, tls));
    }

    open_h2_coalesced(pool, connector, h2_config, key, host, port, proxy).await
}

/// Obtain a cloneable [`crate::quic::H3Client`] for `(host, port)`, reusing a
/// live pooled QUIC connection when one exists and otherwise driving a fresh
/// QUIC + HTTP/3 handshake. H3 has no proxy support, so the key proxy is
/// always `None`.
///
/// Connection-only: returns once the connection is usable, before any request
/// stream is opened. The transport sends the request on the returned handle;
/// the [`ProtocolPolicy::Race`](crate::ProtocolPolicy) path races this against
/// [`checkout_handle`] (H2) and sends on whichever connection comes up first.
#[cfg(feature = "http3")]
pub async fn checkout_h3_handle(
    pool: &Arc<Pool>,
    h3_config: &crate::quic::H3Config,
    profile: &crate::profile::BrowserProfile,
    host: &str,
    port: u16,
) -> Result<(crate::quic::H3Client, TlsInfo), crate::Error> {
    let key = make_key(host, port, None);

    pool.evict_idle();

    if let Some(hit) = pool.checkout_h3(&key) {
        return Ok(hit);
    }

    let (handle, driver, tls) = crate::quic::open_fresh_h3(h3_config, profile, host, port)
        .await
        .map_err(crate::Error::Http3)?;
    Ok(pool.install_or_get_h3(key, handle, driver, tls))
}

/// Send an HTTP/3 request, reusing a pooled QUIC connection when alive.
///
/// A pool hit may be stale (the connection died since last use). The request
/// is retried once on a guaranteed-fresh connection **only** when the failure
/// proves the request never left the client (`H3SendError::NotSent`) — the
/// quintessential stale-idle case. A failure
/// that may have reached the origin (stream reset, mid-response loss) is
/// surfaced as-is, never replayed, so a non-idempotent request is never sent
/// to the origin twice. A fresh connection's failure is likewise never retried.
#[cfg(feature = "http3")]
#[allow(clippy::too_many_arguments)]
pub async fn send_request_h3_pooled(
    pool: &Arc<Pool>,
    h3_config: &crate::quic::H3Config,
    profile: &crate::profile::BrowserProfile,
    host: &str,
    port: u16,
    method: &str,
    authority: &str,
    path: &str,
    headers: &[(String, String)],
    body: Option<bytes::Bytes>,
) -> Result<(crate::quic::H3Response, TlsInfo), crate::Error> {
    let key = make_key(host, port, None);

    pool.evict_idle();

    if let Some((handle, tls)) = pool.checkout_h3(&key) {
        match handle
            .send_request(method, authority, path, headers, body.clone())
            .await
        {
            Ok(resp) => return Ok((resp, tls)),
            // Ambiguous failure (may have reached the origin): do not replay.
            Err(e) if !e.is_retryable() => {
                return Err(crate::Error::Http3(e.message().to_string()));
            }
            Err(e) => {
                tracing::info!(
                    target: "leyline::pool",
                    host = %key.host,
                    port = key.port,
                    error = %e.message(),
                    "pool stale hit -- pooled h3 connection unsent, opening fresh"
                );
                pool.invalidate(&key);
            }
        }
    }

    let (handle, driver, tls) = crate::quic::open_fresh_h3(h3_config, profile, host, port)
        .await
        .map_err(crate::Error::Http3)?;
    let (handle, tls) = pool.install_or_get_h3(key, handle, driver, tls);
    let resp = handle
        .send_request(method, authority, path, headers, body)
        .await
        .map_err(|e| crate::Error::Http3(e.message().to_string()))?;
    Ok((resp, tls))
}

/// Send a request, reusing a pooled H2 connection when available.
///
/// If a pooled handle exists for the destination, it is cloned and the
/// request is fired; many concurrent callers share one TCP connection.
/// If the pooled driver has died, or the request fails with a
/// connection-level error, the entry is invalidated and a fresh TLS +
/// H2 connection is established for a single retry.
#[tracing::instrument(
    name = "pool.send_request",
    level = "debug",
    skip_all,
    fields(
        http.authority = pseudo.authority.as_str(),
        http.method = pseudo.method.as_str(),
        pool.hit = tracing::field::Empty,
    )
)]
#[allow(clippy::too_many_arguments)]
pub async fn send_request(
    pool: &Arc<Pool>,
    connector: &ConnectorVariant,
    h2_config: &H2Config,
    pseudo: PseudoHeaders,
    headers: Vec<(String, String)>,
    body: RequestBody,
    proxy: Option<&str>,
    stream_response: bool,
) -> Result<(H2ResponseEx, TlsInfo), crate::Error> {
    let host = &pseudo.authority;
    let port = if pseudo.scheme == "https" { 443 } else { 80 };

    let (connect_host, connect_port) = parse_authority(host, port);

    let key = make_key(connect_host, connect_port, proxy);

    pool.evict_idle();

    let body_is_stream = matches!(body, RequestBody::Streaming { .. });
    let retry_buf: Option<bytes::Bytes> = match &body {
        RequestBody::Buffered(b) => Some(b.clone()),
        _ => None,
    };
    let mut body = body;

    if let Some((handle, tls)) = pool.checkout_h2(&key) {
        let pooled_body = std::mem::replace(&mut body, RequestBody::None);
        match handle
            .send_request_ex(
                pseudo.clone(),
                headers.clone(),
                pooled_body,
                stream_response,
            )
            .await
        {
            Ok(resp) => {
                tracing::Span::current().record("pool.hit", true);
                return Ok((resp, tls));
            }
            Err(e) => {
                // Pool returned an entry the checkout-side `is_dead`
                // probe couldn't catch — the connection looked alive but
                // the next request errored at the transport layer. This
                // is the "silent stale hit" failure mode and the signal
                // a tuned `pool_idle_timeout` should be evaluated against.
                // Emitted at `info` so callers running at default filter
                // levels can grep for it without flipping pool logging
                // to debug.
                tracing::info!(
                    target: "leyline::pool",
                    host = %key.host,
                    port = key.port,
                    proxied = key.proxy.is_some(),
                    error = %e,
                    "pool stale hit -- pooled h2 connection failed mid-request, opening fresh"
                );
                pool.invalidate(&key);
                if body_is_stream {
                    return Err(crate::Error::Body(format!(
                        "pooled connection died and streaming body cannot be retried: {e}"
                    )));
                }
                // The retry path is the ambiguous "silent stale hit" — a handle
                // that looked alive at checkout but failed mid-request, where it
                // is unknown whether the origin processed the request. Replay
                // only when it is provably safe: the server signalled
                // REFUSED_STREAM (RFC 9113 §7 — not processed), or the method is
                // idempotent (RFC 9110 §9.2.2). Otherwise surface the error
                // rather than risk executing a non-idempotent request twice.
                let refused = matches!(
                    &e,
                    crate::h2::H2Error::Stream {
                        code: crate::h2::ErrorCode::RefusedStream,
                        ..
                    }
                );
                if !refused && !crate::core::retry::is_idempotent(&pseudo.method) {
                    return Err(crate::Error::Http2(e));
                }
                if let Some(buf) = &retry_buf {
                    body = RequestBody::Buffered(buf.clone());
                }
            }
        }
    }
    tracing::Span::current().record("pool.hit", false);

    let (handle, tls) = open_h2_coalesced(
        pool,
        connector,
        h2_config,
        key.clone(),
        connect_host,
        connect_port,
        proxy,
    )
    .await?;

    let resp = match handle
        .send_request_ex(pseudo, headers, body, stream_response)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            pool.invalidate(&key);
            return Err(crate::Error::Http2(e));
        }
    };

    Ok((resp, tls))
}

/// Parse `host[:port]` authority into `(host, port)`.
fn parse_authority(authority: &str, default_port: u16) -> (&str, u16) {
    if let Some(colon) = authority.rfind(':') {
        let port_str = &authority[colon + 1..];
        if let Ok(p) = port_str.parse::<u16>() {
            return (&authority[..colon], p);
        }
    }
    (authority, default_port)
}
