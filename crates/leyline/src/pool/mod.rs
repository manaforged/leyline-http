//! HTTP connection pool.

#![forbid(unsafe_code)]
use std::sync::Arc;
use std::time::Instant;

use crate::Error;
use crate::core::ResponseTiming;
use crate::core::retry::is_idempotent;
use crate::h2::client::{H2Client, H2ResponseEx, RequestBody};
use crate::h2::config::H2Config;
use crate::h2::connection::{ClientConnection, HeaderPair, PseudoHeaders};
use crate::h2::{ErrorCode, H2Error};
#[cfg(feature = "http3")]
use crate::profile::BrowserProfile;
#[cfg(feature = "http3")]
use crate::quic::{H3Client, H3Config, H3RequestBodyStream, H3ResponseParts, open_fresh_h3};
#[cfg(feature = "http3")]
use crate::tls::TlsTrustConfig;
use crate::tls::{FingerprintConnector, TlsError};

mod h1;
#[expect(
    clippy::module_inception,
    reason = "pool::pool is the pool engine; the parent module is the public facade"
)]
mod pool;
mod types;

pub use h1::{
    H1Body, H1Io, H1PooledError, H1Response, H1ResponseBody, H1Target, MAX_H1_BODY_BYTES,
    MAX_H1_HEADER_BYTES, send_request_h1_pooled,
};
pub use pool::{
    DEFAULT_IDLE_TIMEOUT, DEFAULT_MAX_CONNECTIONS, DEFAULT_MAX_H1_CONNS_PER_HOST, Pool,
};
pub use types::{H1Slot, PoolStats, TlsInfo};

use types::{H2Io, PoolKey, Transport};

/// Construct a pool key for a transport family.
pub(crate) fn make_key(
    scheme: &str,
    host: &str,
    port: u16,
    proxy: Option<&str>,
    transport: Transport,
) -> PoolKey {
    PoolKey {
        scheme: scheme.to_string(),
        host: host.to_string(),
        port,
        proxy: proxy.map(|s| s.to_string()),
        transport,
    }
}

/// Establish a fresh TLS + H2 connection to `(host, port, proxy)`, require that ALPN negotiated `h2`, install the driver into `pool` under `key`, and return the cloneable client handle plus its TLS metadata.
async fn open_fresh_h2(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    key: PoolKey,
    host: &str,
    port: u16,
    proxy: Option<&str>,
) -> Result<(H2Client, TlsInfo), Error> {
    let tls_stream = connector
        .connect(host, port, proxy)
        .await
        .map_err(Error::from)?;
    if tls_stream.alpn.as_deref() != Some(b"h2") {
        let negotiated = tls_stream
            .alpn
            .as_ref()
            .map(|p| String::from_utf8_lossy(p).to_string())
            .unwrap_or_else(|| "none".to_string());
        return Err(Error::AlpnMismatch { negotiated });
    }

    let tls = TlsInfo {
        peer_cert_der: tls_stream.peer_cert_der.clone(),
        version: tls_stream.tls_version.clone(),
        cipher: tls_stream.tls_cipher.clone(),
    };
    let (handle, driver) = ClientConnection::<H2Io>::start(tls_stream.stream, h2_config.clone())
        .await
        .map_err(Error::from)?;

    Ok(pool.install_h2(key, handle, driver, tls))
}

/// Reconstruct an owned [`crate::Error`] from an `Arc`-shared coalesced-connect failure.
fn connect_err(err: &Error) -> Error {
    match err {
        Error::Tls(err) => Error::Tls(match err {
            TlsError::SslConfig(msg) => TlsError::SslConfig(msg.clone()),
            TlsError::Handshake(msg) => TlsError::Handshake(msg.clone()),
            TlsError::HandshakeIo(err) => {
                TlsError::HandshakeIo(std::io::Error::new(err.kind(), err.to_string()))
            }
            TlsError::Certificate(msg) => TlsError::Certificate(msg.clone()),
            TlsError::Hostname(msg) => TlsError::Hostname(msg.clone()),
            TlsError::Pinning(msg) => TlsError::Pinning(msg.clone()),
            TlsError::TcpConnect(err) => {
                TlsError::TcpConnect(std::io::Error::new(err.kind(), err.to_string()))
            }
            TlsError::Dns(err) => TlsError::Dns(std::io::Error::new(err.kind(), err.to_string())),
            TlsError::SslConnect(msg) => TlsError::SslConnect(msg.clone()),
            TlsError::Profile(msg) => TlsError::Profile(msg.clone()),
            TlsError::TrustStore(msg) => TlsError::TrustStore(msg.clone()),
        }),
        Error::Io(err) => Error::Io(std::io::Error::new(err.kind(), err.to_string())),
        Error::AlpnMismatch { negotiated } => Error::AlpnMismatch {
            negotiated: negotiated.clone(),
        },
        Error::Http3(msg) => Error::Http3(msg.clone()),
        other => Error::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            other.to_string(),
        )),
    }
}

fn h2_inflight_connect(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    key: &PoolKey,
    host: &str,
    port: u16,
    proxy: Option<&str>,
) -> self::pool::SharedConnect {
    use futures_util::FutureExt;

    pool.inflight_h2_get_or_insert_with(key.clone(), || {
        let pool = Arc::clone(pool);
        let connector = connector.clone();
        let h2_config = h2_config.clone();
        let connect_key = key.clone();
        let cleanup_key = key.clone();
        let host = host.to_string();
        let proxy = proxy.map(|s| s.to_string());
        let handle = tokio::spawn(async move {
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
            pool.inflight_h2_remove(&cleanup_key);
            result
        });
        async move {
            handle.await.unwrap_or_else(|e| {
                Err(Arc::new(Error::Http2(H2Error::Connection {
                    code: ErrorCode::InternalError,
                    reason: format!("h2 connect task failed: {e}"),
                })))
            })
        }
        .boxed()
        .shared()
    })
}

async fn open_h2_coalesced(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    key: PoolKey,
    host: &str,
    port: u16,
    proxy: Option<&str>,
) -> Result<(H2Client, TlsInfo), Error> {
    if let Some(hit) = pool.checkout_h2(&key) {
        return Ok(hit);
    }

    let mut last_err: Option<Arc<Error>> = None;
    for attempt in 0..2u8 {
        if attempt > 0 {
            if let Some(hit) = pool.checkout_h2(&key) {
                return Ok(hit);
            }
        }
        let shared = h2_inflight_connect(pool, connector, h2_config, &key, host, port, proxy);
        match shared.await {
            Ok(pair) => return Ok(pair),
            Err(e) => last_err = Some(e),
        }
    }
    Err(connect_err(
        &last_err.expect("the retry loop runs at least once"),
    ))
}

/// Obtain a cloneable [`crate::h2::H2Client`] handle for `(host, port, proxy)`, reusing an existing pooled connection when available and otherwise establishing a fresh TLS + H2 handshake.
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
    let key = make_key("https", host, port, proxy, Transport::Tcp);

    pool.evict_idle();

    if let Some((handle, tls)) = pool.checkout_h2(&key) {
        return Ok((handle, tls));
    }

    open_h2_coalesced(pool, connector, h2_config, key, host, port, proxy).await
}

/// Obtain a cloneable `H3Client` for `(host, port)`, reusing a live pooled QUIC connection when one exists and otherwise driving a fresh handshake.
#[cfg(feature = "http3")]
#[tracing::instrument(
    name = "pool.checkout_h3_handle",
    level = "debug",
    skip_all,
    fields(host, port)
)]
pub async fn checkout_h3_handle(
    pool: &Arc<Pool>,
    h3_config: &H3Config,
    profile: &BrowserProfile,
    trust: &TlsTrustConfig,
    host: &str,
    port: u16,
) -> Result<(H3Client, TlsInfo), Error> {
    let key = make_key("https", host, port, None, Transport::Quic);

    pool.evict_idle();

    if let Some(hit) = pool.checkout_h3(&key) {
        return Ok(hit);
    }

    open_h3_coalesced(pool, h3_config, profile, trust, key, host, port).await
}

/// Open a fresh QUIC + HTTP/3 connection and install it into `pool` under `key`, returning the cloneable handle plus TLS metadata.
#[cfg(feature = "http3")]
async fn open_fresh_h3_installed(
    pool: &Arc<Pool>,
    h3_config: &H3Config,
    profile: &BrowserProfile,
    trust: &TlsTrustConfig,
    key: PoolKey,
    host: &str,
    port: u16,
) -> Result<(H3Client, TlsInfo), Error> {
    let (handle, driver, tls) = open_fresh_h3(h3_config, profile, trust, host, port)
        .await
        .map_err(Error::Http3)?;
    Ok(pool.install_or_get_h3(key, handle, driver, tls))
}

#[cfg(feature = "http3")]
fn h3_inflight_connect(
    pool: &Arc<Pool>,
    h3_config: &H3Config,
    profile: &BrowserProfile,
    trust: &TlsTrustConfig,
    key: &PoolKey,
    host: &str,
    port: u16,
) -> self::pool::SharedH3Connect {
    use futures_util::FutureExt;

    pool.inflight_h3_get_or_insert_with(key.clone(), || {
        let pool = Arc::clone(pool);
        let h3_config = h3_config.clone();
        let profile = profile.clone();
        let trust = trust.clone();
        let connect_key = key.clone();
        let cleanup_key = key.clone();
        let host = host.to_string();
        let handle = tokio::spawn(async move {
            let result = open_fresh_h3_installed(
                &pool,
                &h3_config,
                &profile,
                &trust,
                connect_key,
                &host,
                port,
            )
            .await
            .map_err(Arc::new);
            pool.inflight_h3_remove(&cleanup_key);
            result
        });
        async move {
            handle.await.unwrap_or_else(|e| {
                Err(Arc::new(Error::Http3(format!(
                    "h3 connect task failed: {e}"
                ))))
            })
        }
        .boxed()
        .shared()
    })
}

#[cfg(feature = "http3")]
async fn open_h3_coalesced(
    pool: &Arc<Pool>,
    h3_config: &H3Config,
    profile: &BrowserProfile,
    trust: &TlsTrustConfig,
    key: PoolKey,
    host: &str,
    port: u16,
) -> Result<(H3Client, TlsInfo), Error> {
    if let Some(hit) = pool.checkout_h3(&key) {
        return Ok(hit);
    }

    let mut last_err: Option<Arc<Error>> = None;
    for attempt in 0..2u8 {
        if attempt > 0 {
            if let Some(hit) = pool.checkout_h3(&key) {
                return Ok(hit);
            }
        }
        let shared = h3_inflight_connect(pool, h3_config, profile, trust, &key, host, port);
        match shared.await {
            Ok(pair) => return Ok(pair),
            Err(e) => last_err = Some(e),
        }
    }
    Err(connect_err(
        &last_err.expect("the retry loop runs at least once"),
    ))
}

/// Send an HTTP/3 request, reusing a pooled QUIC connection when alive.
#[cfg(feature = "http3")]
#[expect(
    clippy::too_many_arguments,
    reason = "flat per-request wire fields across one internal call path"
)]
pub async fn send_request_h3_pooled(
    pool: &Arc<Pool>,
    h3_config: &H3Config,
    profile: &BrowserProfile,
    trust: &TlsTrustConfig,
    host: &str,
    port: u16,
    method: &str,
    authority: &str,
    path: &str,
    headers: &[(String, String)],
    body: Option<bytes::Bytes>,
    body_stream: Option<H3RequestBodyStream>,
    stream_response: bool,
) -> Result<(H3ResponseParts, TlsInfo), Error> {
    let key = make_key("https", host, port, None, Transport::Quic);

    pool.evict_idle();

    let body_is_stream = body_stream.is_some();
    let mut body_stream = body_stream;

    if let Some((handle, tls)) = pool.checkout_h3(&key) {
        match handle
            .send_request(
                method,
                authority,
                path,
                headers,
                body.clone(),
                body_stream.take(),
                stream_response,
            )
            .await
        {
            Ok(resp) => return Ok((resp, tls)),
            Err(e) if !e.is_retryable() => {
                return Err(Error::Http3(e.message().to_string()));
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
                if body_is_stream {
                    return Err(Error::Body(format!(
                        "pooled h3 connection died and a streaming request body cannot be retried: {}",
                        e.message()
                    )));
                }
            }
        }
    }

    let (handle, tls) = open_h3_coalesced(pool, h3_config, profile, trust, key, host, port).await?;
    let resp = handle
        .send_request(
            method,
            authority,
            path,
            headers,
            body,
            body_stream,
            stream_response,
        )
        .await
        .map_err(|e| Error::Http3(e.message().to_string()))?;
    Ok((resp, tls))
}

/// Send a request, reusing a pooled H2 connection when available.
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
#[expect(
    clippy::too_many_arguments,
    reason = "flat per-request wire fields across one internal call path"
)]
pub async fn send_request(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    pseudo: PseudoHeaders,
    headers: Vec<HeaderPair>,
    body: RequestBody,
    proxy: Option<&str>,
    stream_response: bool,
) -> Result<(H2ResponseEx, TlsInfo, ResponseTiming), Error> {
    let started = Instant::now();
    let host = &pseudo.authority;
    let port = if pseudo.scheme == "https" { 443 } else { 80 };

    let (connect_host, connect_port) = parse_authority(host, port);

    let key = make_key(
        &pseudo.scheme,
        connect_host,
        connect_port,
        proxy,
        Transport::Tcp,
    );

    pool.evict_idle();

    let body_is_stream = matches!(body, RequestBody::Streaming { .. });
    let retry_buf: Option<bytes::Bytes> = match &body {
        RequestBody::Buffered(b) => Some(b.clone()),
        _ => None,
    };
    let mut body = body;

    if let Some((handle, tls)) = pool.checkout_h2(&key) {
        let pooled_body = std::mem::replace(&mut body, RequestBody::None);
        let send_started = Instant::now();
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
                let send_ms = ms_since(send_started);
                let timing = ResponseTiming {
                    reused: true,
                    connect_ms: None,
                    send_ms,
                    total_ms: ms_since(started),
                };
                return Ok((resp, tls, timing));
            }
            Err(e) => {
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
                    return Err(Error::Body(format!(
                        "pooled connection died and streaming body cannot be retried: {e}"
                    )));
                }
                let refused = matches!(
                    &e,
                    H2Error::Stream {
                        code: ErrorCode::RefusedStream,
                        ..
                    }
                );
                if !refused && !is_idempotent(&pseudo.method) {
                    return Err(Error::Http2(e));
                }
                if let Some(buf) = &retry_buf {
                    body = RequestBody::Buffered(buf.clone());
                }
            }
        }
    }
    tracing::Span::current().record("pool.hit", false);

    let connect_started = Instant::now();
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
    let connect_ms = ms_since(connect_started);

    let send_started = Instant::now();
    let resp = match handle
        .send_request_ex(pseudo, headers, body, stream_response)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            pool.invalidate(&key);
            return Err(Error::Http2(e));
        }
    };

    let timing = ResponseTiming {
        reused: false,
        connect_ms: Some(connect_ms),
        send_ms: ms_since(send_started),
        total_ms: ms_since(started),
    };
    Ok((resp, tls, timing))
}

/// Milliseconds elapsed since `start`, saturating into `u32` (a hop that somehow runs longer than ~49 days clamps rather than wraps).
fn ms_since(start: Instant) -> u32 {
    u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX)
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

#[cfg(test)]
mod tests;
