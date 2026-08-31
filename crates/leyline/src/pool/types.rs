//! Pool-internal types: keys, connection entries, TLS metadata, and stats.

use std::collections::VecDeque;
use std::sync::atomic::AtomicU64;
use std::time::Instant;

use crate::h2::client::{DriverTask, H2Client};
#[cfg(feature = "http3")]
use crate::quic::{H3Client, H3DriverTask};

use crate::pool::h1::H1Io;
use crate::tls::TlsIo;

/// I/O type used during the H2 handshake — the backend-agnostic TLS stream ([`TlsIo`]).
pub(crate) type H2Io = TlsIo;

/// TLS handshake result carried alongside pooled connections.
#[derive(Clone, Default)]
pub struct TlsInfo {
    /// Peer certificate in DER encoding, if the peer presented one.
    pub peer_cert_der: Option<Vec<u8>>,
    /// Negotiated TLS protocol version (e.g. `"TLSv1.3"`).
    pub version: Option<String>,
    /// Negotiated TLS cipher suite name.
    pub cipher: Option<String>,
}

/// Transport family a pool entry rides on.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) enum Transport {
    Tcp,
    #[cfg_attr(not(feature = "http3"), allow(dead_code))]
    Quic,
}

/// Pool key — uniquely identifies a destination and its transport family.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(crate) struct PoolKey {
    pub(crate) host: String,
    pub(crate) port: u16,
    /// Request scheme — part of the key because a plaintext socket must never satisfy a later `https://` request to the same host:port (an attacker serving plaintext on 443 would otherwise have his socket pooled for the TLS request, credentials included).
    pub(crate) scheme: String,
    pub(crate) proxy: Option<String>,
    pub(crate) transport: Transport,
}

/// An owned HTTP/1.1 keep-alive slot.
pub struct H1Slot {
    /// TLS over TCP for `https://`; plaintext TCP for `http://`.
    pub(crate) io: Box<dyn H1Io>,
}

/// A pooled connection entry.
pub(crate) enum PooledConn {
    /// An HTTP/2 entry.
    H2 {
        handle: H2Client,
        /// Wrapped in `Option` so we can take it on eviction.
        _driver: Option<DriverTask>,
        last_use: Instant,
        tls: TlsInfo,
    },
    /// An HTTP/1.1 keep-alive entry.
    H1 {
        idle: VecDeque<(H1Slot, Instant)>,
        last_use: Instant,
        tls: TlsInfo,
    },
    /// An HTTP/3 entry.
    #[cfg(feature = "http3")]
    H3 {
        handle: H3Client,
        /// Wrapped in `Option` so we can take it on eviction.
        _driver: Option<H3DriverTask>,
        last_use: Instant,
        tls: TlsInfo,
    },
}

impl PooledConn {
    pub(crate) fn last_use(&self) -> Instant {
        match self {
            PooledConn::H2 { last_use, .. } | PooledConn::H1 { last_use, .. } => *last_use,
            #[cfg(feature = "http3")]
            PooledConn::H3 { last_use, .. } => *last_use,
        }
    }

    pub(crate) fn set_last_use(&mut self, now: Instant) {
        match self {
            PooledConn::H2 { last_use, .. } | PooledConn::H1 { last_use, .. } => *last_use = now,
            #[cfg(feature = "http3")]
            PooledConn::H3 { last_use, .. } => *last_use = now,
        }
    }

    /// True when the entry should be evicted rather than handed out.
    pub(crate) fn is_dead(&self) -> bool {
        match self {
            PooledConn::H2 { handle, .. } => handle.is_closed(),
            PooledConn::H1 { idle, .. } => idle.is_empty(),
            #[cfg(feature = "http3")]
            PooledConn::H3 { handle, .. } => handle.is_closed(),
        }
    }
}

/// Observability snapshot of a [`super::Pool`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PoolStats {
    /// Live entries in the pool at observation time (H1 + H2 combined).
    pub entries: usize,
    /// Max-connections cap the pool is enforcing.
    pub max_connections: usize,
    /// Cumulative successful H2 checkouts (handle was alive + reused).
    pub h2_hits: u64,
    /// Cumulative H2 checkout misses (no entry, or entry was dead).
    pub h2_misses: u64,
    /// Cumulative H1 checkouts that popped a pooled connection.
    pub h1_hits: u64,
    /// Cumulative H1 checkout misses (no entry, dead slot, or idle window exceeded).
    pub h1_misses: u64,
    /// Cumulative successful H3 checkouts (QUIC connection alive + reused).
    pub h3_hits: u64,
    /// Cumulative H3 checkout misses (no entry, or entry was dead).
    pub h3_misses: u64,
    /// Cumulative evictions from the idle timeout sweep.
    pub evictions_idle: u64,
    /// Cumulative evictions triggered by the LRU cap.
    pub evictions_lru: u64,
    /// Cumulative evictions caused by a send failing with a connection-level error mid-exchange — the residual probe-to-write race a checkout liveness probe cannot close.
    pub evictions_dead: u64,
    /// Cumulative H1 connections the checkout liveness probe found already dead (peer-closed / EOF / pre-request desync) and discarded before use.
    pub stale_probed: u64,
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
    pub(crate) h3_hits: AtomicU64,
    pub(crate) h3_misses: AtomicU64,
    pub(crate) evictions_idle: AtomicU64,
    pub(crate) evictions_lru: AtomicU64,
    pub(crate) evictions_dead: AtomicU64,
    pub(crate) stale_probed: AtomicU64,
    pub(crate) installs: AtomicU64,
}
