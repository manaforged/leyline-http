//! HTTP/2 connection pool.
//!
//! Keeps one H2 connection per (host, port, proxy) key. H2 multiplexes
//! requests over a single TCP connection, so one is enough per destination.
//! The pooled handle is [`H2Client`], which is cloneable and multiplexes
//! concurrent requests internally — checkout is a cheap `clone`, and many
//! in-flight requests share the same TCP connection without serialization.
//!
//! ## Eviction
//! - **Idle timeout**: entries untouched for longer than the configured
//!   window (default 90 s) are dropped.
//! - **LRU cap**: the pool carries at most `max_connections` entries
//!   (default 256). When inserting a new entry would exceed the cap, the
//!   least-recently-used entry is evicted. Dropping its `DriverTask`
//!   triggers a graceful GOAWAY.
//!
//! The last-use timestamp is updated on every `checkout`, so LRU ordering
//! reflects actual request activity, not install time.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::net::TcpStream;

use leyline_h2::client::{DriverTask, H2Client, H2ResponseEx, RequestBody};
use leyline_h2::config::H2Config;
use leyline_h2::connection::{ClientConnection, PseudoHeaders};
use leyline_tls::FingerprintConnector;

/// Concrete I/O type used during handshake — BoringSSL over TCP.
type H2Io = tokio_boring::SslStream<TcpStream>;

/// TLS handshake result carried alongside pooled H2 connections.
/// Every field is per-connection (it's the same for every request
/// that reuses the connection).
#[derive(Clone, Default)]
pub struct TlsInfo {
    /// Peer certificate in DER encoding, if the peer presented one.
    pub peer_cert_der: Option<Vec<u8>>,
    /// Negotiated TLS protocol version (e.g. `"TLSv1.3"`).
    pub version: Option<String>,
    /// Negotiated TLS cipher suite name.
    pub cipher: Option<String>,
}

/// A pooled connection entry.
///
/// The `H2Client` is cloneable; each outbound request clones it, so the
/// pool can hand out unlimited concurrent handles without ever checking
/// the connection out / in. The `DriverTask` stays here so dropping the
/// pool entry forces a graceful shutdown.
struct PooledConn {
    handle: H2Client,
    /// Wrapped in `Option` so we can take it on eviction and keep the
    /// driver running only as long as the entry is alive.
    _driver: Option<DriverTask>,
    last_use: Instant,
    tls: TlsInfo,
}

/// Pool key — uniquely identifies a destination.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct PoolKey {
    host: String,
    port: u16,
    proxy: Option<String>,
}

/// HTTP/2 connection pool.
///
/// Thread-safe (`Arc<Mutex<>>`) — a `Session` holds `Arc<Pool>` so cloned
/// sessions share the same pool.
pub struct Pool {
    inner: Mutex<HashMap<PoolKey, PooledConn>>,
    idle_timeout: Duration,
    /// LRU cap. Insertions beyond this count evict the least-recently-used
    /// entry (dropping its `DriverTask` triggers a graceful GOAWAY).
    max_connections: usize,
}

/// Default idle-timeout for pooled connections.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// Default LRU cap. Chosen for a "a few hundred distinct destinations"
/// workload — raise it for workloads with many hosts.
pub const DEFAULT_MAX_CONNECTIONS: usize = 256;

impl Pool {
    /// Create a pool with default 90 s idle timeout and a 256-entry LRU cap.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            max_connections: DEFAULT_MAX_CONNECTIONS,
        }
    }

    /// Create a pool with a custom idle timeout (LRU cap stays default).
    pub fn with_idle_timeout(timeout: Duration) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            idle_timeout: timeout,
            max_connections: DEFAULT_MAX_CONNECTIONS,
        }
    }

    /// Create a pool with explicit idle timeout and LRU cap.
    pub fn with_limits(idle_timeout: Duration, max_connections: usize) -> Self {
        assert!(max_connections > 0, "max_connections must be at least 1");
        Self {
            inner: Mutex::new(HashMap::new()),
            idle_timeout,
            max_connections,
        }
    }

    /// Evict idle connections AND any entry whose driver has already
    /// shut down. Called before checkout.
    fn evict_idle(&self) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        map.retain(|_, entry| {
            !entry.handle.is_closed() && now.duration_since(entry.last_use) < self.idle_timeout
        });
    }

    /// Drop the least-recently-used entry if we're at or over cap. Called
    /// from `install` before inserting.
    fn evict_lru_if_needed(map: &mut HashMap<PoolKey, PooledConn>, cap: usize) {
        while map.len() >= cap {
            let victim = map
                .iter()
                .min_by_key(|(_, e)| e.last_use)
                .map(|(k, _)| k.clone());
            match victim {
                Some(k) => {
                    map.remove(&k);
                }
                None => break,
            }
        }
    }

    /// Look up a live handle for `key`, touching its last-use timestamp.
    /// Returns `None` if no entry exists, or if the driver has shut down
    /// (in which case the dead entry is removed).
    fn checkout(&self, key: &PoolKey) -> Option<(H2Client, TlsInfo)> {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let is_dead = map.get(key).map(|e| e.handle.is_closed()).unwrap_or(false);
        if is_dead {
            map.remove(key);
            return None;
        }
        map.get_mut(key).map(|entry| {
            entry.last_use = Instant::now();
            (entry.handle.clone(), entry.tls.clone())
        })
    }

    /// Install a new pooled connection, replacing any existing entry and
    /// LRU-evicting the oldest if the cap would be exceeded.
    fn install(&self, key: PoolKey, handle: H2Client, driver: DriverTask, tls: TlsInfo) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        // Only LRU-evict if we're adding a net-new key. Replacing is free.
        if !map.contains_key(&key) {
            Self::evict_lru_if_needed(&mut map, self.max_connections);
        }
        map.insert(
            key,
            PooledConn {
                handle,
                _driver: Some(driver),
                last_use: Instant::now(),
                tls,
            },
        );
    }

    /// Current entry count. Useful for tests and observability.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// True when the pool has no entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drop the entry for `key` if it's there. Used when a send fails
    /// with a connection-level error and we want a fresh driver next time.
    fn invalidate(&self, key: &PoolKey) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        map.remove(key);
    }
}

impl Default for Pool {
    fn default() -> Self {
        Self::new()
    }
}

/// Obtain a cloneable [`H2Client`] handle for `(host, port, proxy)`,
/// reusing an existing pooled connection when available and otherwise
/// establishing a fresh TLS + H2 handshake.
///
/// Unlike [`send_request`] this helper does not issue a request — it
/// exists so callers like `Session::websocket` can open an extended
/// CONNECT (RFC 8441) stream on the same connection the rest of the
/// session's HTTP traffic rides over. The returned [`TlsInfo`] is a
/// snapshot of the TLS handshake for the underlying connection.
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
) -> Result<(H2Client, TlsInfo), String> {
    let key = PoolKey {
        host: host.to_string(),
        port,
        proxy: proxy.map(|s| s.to_string()),
    };

    pool.evict_idle();

    if let Some((handle, tls)) = pool.checkout(&key) {
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

    pool.install(key, handle.clone(), driver, tls.clone());
    Ok((handle, tls))
}

/// Send a request, reusing a pooled connection when available.
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

    // Parse host:port if authority contains port.
    let (connect_host, connect_port) = if let Some(colon) = host.rfind(':') {
        let port_str = &host[colon + 1..];
        if let Ok(p) = port_str.parse::<u16>() {
            (&host[..colon], p)
        } else {
            (host.as_str(), port)
        }
    } else {
        (host.as_str(), port)
    };

    let key = PoolKey {
        host: connect_host.to_string(),
        port: connect_port,
        proxy: proxy.map(|s| s.to_string()),
    };

    pool.evict_idle();

    // Streaming request bodies are one-shot — no retry possible. For
    // buffered bodies we keep a clone in case the pooled attempt fails
    // and we need to retry on a fresh connection.
    let body_is_stream = matches!(body, RequestBody::Streaming { .. });
    let retry_buf: Option<bytes::Bytes> = match &body {
        RequestBody::Buffered(b) => Some(b.clone()),
        _ => None,
    };
    let mut body = body;

    // Try pooled connection first.
    if let Some((handle, tls)) = pool.checkout(&key) {
        let pooled_body = std::mem::replace(&mut body, RequestBody::None);
        match handle
            .send_request_ex(pseudo.clone(), headers.clone(), pooled_body, stream_response)
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
                // Fall through to fresh connection; restore retry body.
                if let Some(buf) = &retry_buf {
                    body = RequestBody::Buffered(buf.clone());
                }
            }
        }
    }
    tracing::Span::current().record("pool.hit", false);

    // Fresh connection.
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

    // Install before firing so concurrent callers can join us on the same
    // connection while the first request is in-flight.
    pool.install(key.clone(), handle.clone(), driver, tls.clone());

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
