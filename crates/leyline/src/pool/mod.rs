//! HTTP connection pool.
//!
//! Keys entries by `(host, port, proxy)` and supports two protocol
//! flavours:
//!
//! - **HTTP/2** — one connection per destination; the pooled handle
//!   is [`H2Client`], which is cloneable and multiplexes concurrent
//!   requests over the same TCP connection. Checkout is a cheap
//!   `clone` and many in-flight requests share the connection
//!   without serialisation.
//! - **HTTP/1.1 keep-alive** — one connection per destination, but
//!   single-checkout semantics (H1 cannot multiplex). The owned
//!   stream is taken out of the pool for the duration of a
//!   request/response exchange and reinstated on clean completion
//!   when the response is reusable.
//!
//! ## Eviction
//! - **Idle timeout**: entries untouched for longer than the configured
//!   window (default 90 s) are dropped.
//! - **LRU cap**: the pool carries at most `max_connections` entries
//!   (default 256). H1 and H2 share the cap — no per-protocol limit.
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
use crate::tls::FingerprintConnector;

mod h1;
mod pool;
mod types;

pub use h1::{
    send_request_h1_pooled, H1Body, H1Io, H1PooledError, H1Response, H1ResponseBody, H1Target,
    MAX_H1_BODY_BYTES, MAX_H1_HEADER_BYTES,
};
pub use pool::{Pool, DEFAULT_IDLE_TIMEOUT, DEFAULT_MAX_CONNECTIONS};
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

/// Obtain a cloneable [`H2Client`] handle for `(host, port, proxy)`,
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
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    host: &str,
    port: u16,
    proxy: Option<&str>,
) -> Result<(crate::h2::client::H2Client, TlsInfo), String> {
    let key = make_key(host, port, proxy);

    pool.evict_idle();

    if let Some((handle, tls)) = pool.checkout_h2(&key) {
        return Ok((handle, tls));
    }

    let tls_stream = connector
        .connect(host, port, proxy)
        .await
        .map_err(|e| format!("tls: {e}"))?;
    if tls_stream.alpn.as_deref() != Some(b"h2") {
        let negotiated = tls_stream
            .alpn
            .as_ref()
            .map(|p| String::from_utf8_lossy(p).to_string())
            .unwrap_or_else(|| "none".to_string());
        return Err(format!("alpn: negotiated {negotiated}, expected h2"));
    }

    let tls = TlsInfo {
        peer_cert_der: tls_stream.peer_cert_der.clone(),
        version: tls_stream.tls_version.clone(),
        cipher: tls_stream.tls_cipher.clone(),
    };
    let (handle, driver) = ClientConnection::<H2Io>::start(tls_stream.stream, h2_config.clone())
        .await
        .map_err(|e| format!("h2: {e}"))?;

    pool.install_h2(key, handle.clone(), driver, tls.clone());
    Ok((handle, tls))
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
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    pseudo: PseudoHeaders,
    headers: Vec<(String, String)>,
    body: RequestBody,
    proxy: Option<&str>,
    stream_response: bool,
) -> Result<(H2ResponseEx, TlsInfo), String> {
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
                tracing::debug!(error = %e, "pooled connection dead, opening fresh");
                pool.invalidate(&key);
                if body_is_stream {
                    return Err(format!(
                        "pooled connection died and streaming body cannot be retried: {e}"
                    ));
                }
                if let Some(buf) = &retry_buf {
                    body = RequestBody::Buffered(buf.clone());
                }
            }
        }
    }
    tracing::Span::current().record("pool.hit", false);

    let tls_stream = connector
        .connect(connect_host, connect_port, proxy)
        .await
        .map_err(|e| format!("tls: {e}"))?;
    if tls_stream.alpn.as_deref() != Some(b"h2") {
        let negotiated = tls_stream
            .alpn
            .as_ref()
            .map(|p| String::from_utf8_lossy(p).to_string())
            .unwrap_or_else(|| "none".to_string());
        return Err(format!("alpn: negotiated {negotiated}, expected h2"));
    }

    let tls = TlsInfo {
        peer_cert_der: tls_stream.peer_cert_der.clone(),
        version: tls_stream.tls_version.clone(),
        cipher: tls_stream.tls_cipher.clone(),
    };
    let (handle, driver) = ClientConnection::<H2Io>::start(tls_stream.stream, h2_config.clone())
        .await
        .map_err(|e| format!("h2: {e}"))?;

    pool.install_h2(key.clone(), handle.clone(), driver, tls.clone());

    let resp = match handle
        .send_request_ex(pseudo, headers, body, stream_response)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            pool.invalidate(&key);
            return Err(format!("request: {e}"));
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
