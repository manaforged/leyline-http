//! HTTP/2 connection pool.
//!
//! Keeps one H2 connection per (host, port, proxy) key. H2 multiplexes
//! requests over a single TCP connection, so one is enough per destination.
//! Idle connections are evicted after a configurable timeout.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::net::TcpStream;

use leyline_h2::config::H2Config;
use leyline_h2::connection::{ClientConnection, H2Response, PseudoHeaders};
use leyline_tls::FingerprintConnector;

/// Concrete connection type — BoringSSL over TCP.
type H2Conn = ClientConnection<tokio_boring::SslStream<TcpStream>>;

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

/// A pooled connection with last-use timestamp.
struct PooledConn {
    conn: H2Conn,
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
/// Thread-safe (Arc<Mutex<>>) — a `Session` holds `Arc<Pool>` so cloned
/// sessions share the same pool.
pub struct Pool {
    inner: Mutex<HashMap<PoolKey, PooledConn>>,
    idle_timeout: Duration,
}

impl Pool {
    /// Create a pool with default 90s idle timeout.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            idle_timeout: Duration::from_secs(90),
        }
    }

    /// Create a pool with a custom idle timeout.
    pub fn with_idle_timeout(timeout: Duration) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            idle_timeout: timeout,
        }
    }

    /// Evict idle connections. Called before checkout.
    fn evict_idle(&self) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        map.retain(|_, entry| now.duration_since(entry.last_use) < self.idle_timeout);
    }

    /// Take a connection from the pool, if one exists for this destination.
    fn take(&self, key: &PoolKey) -> Option<(H2Conn, TlsInfo)> {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        map.remove(key).map(|entry| (entry.conn, entry.tls))
    }

    /// Return a connection to the pool for reuse.
    fn put(&self, key: PoolKey, conn: H2Conn, tls: TlsInfo) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        map.insert(
            key,
            PooledConn {
                conn,
                last_use: Instant::now(),
                tls,
            },
        );
    }
}

impl Default for Pool {
    fn default() -> Self {
        Self::new()
    }
}

/// Send a request, reusing a pooled connection when available.
///
/// If a pooled connection exists and the request succeeds, the connection
/// is returned to the pool. If the pooled connection fails (GOAWAY, reset,
/// etc.), a fresh connection is created and tried once.
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
    body: Option<Bytes>,
    proxy: Option<&str>,
) -> Result<(H2Response, TlsInfo), String> {
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

    // Evict stale connections.
    pool.evict_idle();

    // Try pooled connection first.
    if let Some((mut conn, tls)) = pool.take(&key) {
        match conn
            .send_request(pseudo.clone(), headers.clone(), body.clone())
            .await
        {
            Ok(resp) => {
                tracing::Span::current().record("pool.hit", true);
                // Return connection to pool for future requests.
                pool.put(key, conn, tls.clone());
                return Ok((resp, tls));
            }
            Err(e) => {
                tracing::debug!(error = %e, "pooled connection dead, opening fresh");
                // Pooled connection is dead — fall through to new connection.
                // Connection is dropped (not returned to pool).
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
    let mut conn = ClientConnection::handshake(tls_stream.stream, h2_config.clone())
        .await
        .map_err(|e| format!("h2: {e}"))?;

    let resp = conn
        .send_request(pseudo, headers, body)
        .await
        .map_err(|e| format!("request: {e}"))?;

    // Pool the connection for reuse.
    pool.put(key, conn, tls.clone());

    Ok((resp, tls))
}
