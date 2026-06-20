//! Pool-internal types: keys, connection entries, TLS metadata, and stats.

use std::collections::VecDeque;
use std::sync::atomic::AtomicU64;
use std::time::Instant;

use crate::h2::client::{DriverTask, H2Client};

use crate::pool::h1::H1Io;
use crate::tls::TlsIo;

/// I/O type used during the H2 handshake — the backend-agnostic TLS
/// stream ([`TlsIo`]). Today that resolves to BoringSSL over TCP; a
/// future TLS backend slots in as a new `TlsIo` arm without changing
/// this alias or the H2 driver above it.
pub(crate) type H2Io = TlsIo;

/// TLS handshake result carried alongside pooled connections.
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

/// Pool key — uniquely identifies a destination.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(crate) struct PoolKey {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) proxy: Option<String>,
}

/// An owned HTTP/1.1 keep-alive slot. Carries the TLS-over-TCP or
/// plaintext-TCP stream, boxed so the pool can hold many different
/// I/O types behind one key.
pub struct H1Slot {
    /// TLS over TCP for `https://`; plaintext TCP for `http://`.
    /// Boxed rather than parameterising [`Pool`] by I/O type so a
    /// single pool can service both schemes.
    pub(crate) io: Box<dyn H1Io>,
}

/// A pooled connection entry.
pub(crate) enum PooledConn {
    /// An HTTP/2 entry. The `H2Client` is cloneable; each outbound
    /// request clones it, so the pool can hand out unlimited
    /// concurrent handles without ever checking the connection out /
    /// in. The `DriverTask` stays here so dropping the pool entry
    /// forces a graceful shutdown.
    H2 {
        handle: H2Client,
        /// Wrapped in `Option` so we can take it on eviction.
        _driver: Option<DriverTask>,
        last_use: Instant,
        tls: TlsInfo,
    },
    /// An HTTP/1.1 keep-alive entry. Holds a deque of warm idle
    /// connections to this destination. H1 cannot multiplex, so per-host
    /// concurrency comes from several parallel connections (browsers open
    /// up to ~6 per host); a checkout pops one warm connection and a
    /// reusable completion returns it via [`super::Pool::return_h1`]. The
    /// live connection count per host is bounded by the pool's per-host
    /// semaphore (`max_h1_conns_per_host`), so this deque never exceeds
    /// that cap. Each connection carries the `Instant` it was last
    /// returned, for per-connection idle eviction.
    H1 {
        idle: VecDeque<(H1Slot, Instant)>,
        last_use: Instant,
        tls: TlsInfo,
    },
}

impl PooledConn {
    pub(crate) fn last_use(&self) -> Instant {
        match self {
            PooledConn::H2 { last_use, .. } | PooledConn::H1 { last_use, .. } => *last_use,
        }
    }

    pub(crate) fn set_last_use(&mut self, now: Instant) {
        match self {
            PooledConn::H2 { last_use, .. } | PooledConn::H1 { last_use, .. } => *last_use = now,
        }
    }

    /// True when the entry should be evicted rather than handed out.
    pub(crate) fn is_dead(&self) -> bool {
        match self {
            PooledConn::H2 { handle, .. } => handle.is_closed(),
            PooledConn::H1 { idle, .. } => idle.is_empty(),
        }
    }
}

/// Observability snapshot of a [`super::Pool`].
///
/// Returned by [`super::Pool::stats`]. All counters are monotonically
/// increasing over the lifetime of the pool; divide by a wall-clock
/// interval to get rates. `entries` is the instantaneous live-entry
/// count at the moment stats were read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PoolStats {
    /// Live entries in the pool at observation time (H1 + H2 combined).
    pub entries: usize,
    /// Max-connections cap the pool is enforcing.
    pub max_connections: usize,
    /// Cumulative successful H2 checkouts (handle was alive + reused).
    pub h2_hits: u64,
    /// Cumulative H2 checkout misses (no entry, or entry was dead).
    pub h2_misses: u64,
    /// Cumulative H1 checkouts that popped a pooled connection. There is no
    /// liveness probe at checkout, so a counted hit may still prove stale on
    /// use (the exchange then fails and bumps `evictions_dead`); a hit means
    /// "a warm connection was handed out", not "a request succeeded on it".
    pub h1_hits: u64,
    /// Cumulative H1 checkout misses (no entry, dead slot, or idle
    /// window exceeded).
    pub h1_misses: u64,
    /// Cumulative evictions from the idle timeout sweep.
    pub evictions_idle: u64,
    /// Cumulative evictions triggered by the LRU cap.
    pub evictions_lru: u64,
    /// Cumulative evictions caused by a send failing with a
    /// connection-level error.
    pub evictions_dead: u64,
    /// Cumulative fresh connections opened and installed (H1 or H2).
    pub installs: u64,
}

/// Atomic counters backing [`PoolStats`].
#[derive(Default)]
pub(crate) struct PoolCounters {
    pub(crate) h2_hits: AtomicU64,
    pub(crate) h2_misses: AtomicU64,
    pub(crate) h1_hits: AtomicU64,
    pub(crate) h1_misses: AtomicU64,
    pub(crate) evictions_idle: AtomicU64,
    pub(crate) evictions_lru: AtomicU64,
    pub(crate) evictions_dead: AtomicU64,
    pub(crate) installs: AtomicU64,
}
