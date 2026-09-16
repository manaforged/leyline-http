use std::collections::VecDeque;
use std::sync::atomic::AtomicU64;
use std::time::Instant;

use crate::h2::client::{DriverTask, H2Client};
#[cfg(feature = "http3")]
use crate::quic::{H3Client, H3DriverTask};

use crate::pool::h1::H1Io;
use crate::tls::TlsIo;

pub(crate) type H2Io = TlsIo;

#[derive(Clone, Default)]
pub struct TlsInfo {
    pub peer_cert_der: Option<Vec<u8>>,
    pub version: Option<String>,
    pub cipher: Option<String>,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) enum Transport {
    Tcp,
    #[cfg_attr(not(feature = "http3"), allow(dead_code))]
    Quic,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(crate) struct PoolKey {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) scheme: String,
    pub(crate) proxy: Option<String>,
    pub(crate) transport: Transport,
}

pub struct H1Slot {
    pub(crate) io: Box<dyn H1Io>,
}

pub(crate) enum PooledConn {
    H2 {
        handle: H2Client,
        _driver: Option<DriverTask>,
        last_use: Instant,
        tls: TlsInfo,
    },
    H1 {
        idle: VecDeque<(H1Slot, Instant)>,
        last_use: Instant,
        tls: TlsInfo,
    },
    #[cfg(feature = "http3")]
    H3 {
        handle: H3Client,
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

    pub(crate) fn is_dead(&self) -> bool {
        match self {
            PooledConn::H2 { handle, .. } => handle.is_closed(),
            PooledConn::H1 { idle, .. } => idle.is_empty(),
            #[cfg(feature = "http3")]
            PooledConn::H3 { handle, .. } => handle.is_closed(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PoolStats {
    pub entries: usize,
    pub max_connections: usize,
    pub h2_hits: u64,
    pub h2_misses: u64,
    pub h1_hits: u64,
    pub h1_misses: u64,
    pub h3_hits: u64,
    pub h3_misses: u64,
    pub evictions_idle: u64,
    pub evictions_lru: u64,
    pub evictions_dead: u64,
    pub stale_probed: u64,
    pub h2_ping_failures: u64,
    pub installs: u64,
}

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
    pub(crate) h2_ping_failures: AtomicU64,
    pub(crate) installs: AtomicU64,
}
