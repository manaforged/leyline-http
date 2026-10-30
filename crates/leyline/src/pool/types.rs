use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::h2::client::H2Client;
#[cfg(feature = "http3")]
use crate::quic::H3Client;

use crate::pool::h1::H1Io;

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct TlsInfo {
    pub peer_cert_der: Option<Vec<u8>>,
    pub version: Option<String>,
    pub cipher: Option<String>,
}

pub(crate) struct Opened<C> {
    pub(crate) conn: C,
    pub(crate) tls: TlsInfo,
    pub(crate) started: Instant,
    pub(crate) connect_ms: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegKind {
    Pooled,
    Fresh,
}

impl<C> Opened<C> {
    pub(crate) fn pooled((conn, tls): (C, TlsInfo)) -> Self {
        Self {
            conn,
            tls,
            started: Instant::now(),
            connect_ms: None,
        }
    }

    pub(crate) fn fresh((conn, tls): (C, TlsInfo), started: Instant) -> Self {
        Self {
            conn,
            tls,
            started,
            connect_ms: Some(crate::ResponseTiming::millis(started)),
        }
    }

    pub(crate) fn leg(&self) -> LegKind {
        match self.connect_ms {
            Some(_) => LegKind::Fresh,
            None => LegKind::Pooled,
        }
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) enum Transport {
    Tcp,
    #[cfg_attr(
        not(feature = "http3"),
        allow(dead_code, reason = "QUIC transport exists only with http3")
    )]
    Quic,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(crate) struct PoolKey {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) scheme: String,
    pub(crate) proxy: Option<String>,
    pub(crate) transport: Transport,
    pub(crate) partition: u64,
}

pub struct H1Slot {
    pub(crate) io: Box<dyn H1Io>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Proto {
    H1,
    H2,
    H3,
}

#[derive(Clone)]
pub(crate) enum MuxHandle {
    H2(H2Client),
    #[cfg(feature = "http3")]
    H3(H3Client),
}

impl MuxHandle {
    pub(crate) fn is_closed(&self) -> bool {
        match self {
            MuxHandle::H2(handle) => handle.is_closed(),
            #[cfg(feature = "http3")]
            MuxHandle::H3(handle) => handle.is_closed(),
        }
    }

    pub(crate) fn open_streams(&self) -> usize {
        match self {
            MuxHandle::H2(handle) => handle.open_streams(),
            #[cfg(feature = "http3")]
            MuxHandle::H3(handle) => handle.open_streams(),
        }
    }
}

pub(crate) trait MuxClient: Clone {
    const PROTO: Proto;
    fn into_handle(self) -> MuxHandle;
    fn in_handle(handle: &MuxHandle) -> Option<&Self>;
}

impl MuxClient for H2Client {
    const PROTO: Proto = Proto::H2;

    fn into_handle(self) -> MuxHandle {
        MuxHandle::H2(self)
    }

    fn in_handle(handle: &MuxHandle) -> Option<&Self> {
        match handle {
            MuxHandle::H2(handle) => Some(handle),
            #[cfg(feature = "http3")]
            MuxHandle::H3(_) => None,
        }
    }
}

#[cfg(feature = "http3")]
impl MuxClient for H3Client {
    const PROTO: Proto = Proto::H3;

    fn into_handle(self) -> MuxHandle {
        MuxHandle::H3(self)
    }

    fn in_handle(handle: &MuxHandle) -> Option<&Self> {
        match handle {
            MuxHandle::H3(handle) => Some(handle),
            MuxHandle::H2(_) => None,
        }
    }
}

pub(crate) enum PooledConn {
    Mux {
        handle: MuxHandle,
        last_use: Instant,
        tls: TlsInfo,
    },
    H1 {
        idle: VecDeque<(H1Slot, Instant)>,
        last_use: Instant,
        tls: TlsInfo,
    },
}

impl PooledConn {
    pub(crate) fn last_use(&self) -> Instant {
        match self {
            PooledConn::Mux { last_use, .. } | PooledConn::H1 { last_use, .. } => *last_use,
        }
    }

    pub(crate) fn is_dead(&self) -> bool {
        match self {
            PooledConn::Mux { handle, .. } => handle.is_closed(),
            PooledConn::H1 { idle, .. } => idle.is_empty(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConnectionStats {
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
    pub busy: usize,
    pub idle: usize,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Counter {
    Hit(Proto),
    Miss(Proto),
    EvictionsIdle,
    EvictionsLru,
    EvictionsDead,
    StaleProbed,
    H2PingFailures,
    Installs,
}

impl Counter {
    const COUNT: usize = 12;

    fn index(self) -> usize {
        match self {
            Counter::Hit(proto) => proto as usize,
            Counter::Miss(proto) => 3 + proto as usize,
            Counter::EvictionsIdle => 6,
            Counter::EvictionsLru => 7,
            Counter::EvictionsDead => 8,
            Counter::StaleProbed => 9,
            Counter::H2PingFailures => 10,
            Counter::Installs => 11,
        }
    }
}

#[derive(Default)]
pub(crate) struct PoolCounters([AtomicU64; Counter::COUNT]);

impl PoolCounters {
    pub(crate) fn add(&self, counter: Counter, n: u64) {
        self.0[counter.index()].fetch_add(n, Ordering::Relaxed);
    }

    pub(crate) fn checkout(&self, proto: Proto, hit: bool) {
        let counter = if hit {
            Counter::Hit(proto)
        } else {
            Counter::Miss(proto)
        };
        self.add(counter, 1);
    }

    fn get(&self, counter: Counter) -> u64 {
        self.0[counter.index()].load(Ordering::Relaxed)
    }

    pub(crate) fn snapshot(&self) -> ConnectionStats {
        ConnectionStats {
            h2_hits: self.get(Counter::Hit(Proto::H2)),
            h2_misses: self.get(Counter::Miss(Proto::H2)),
            h1_hits: self.get(Counter::Hit(Proto::H1)),
            h1_misses: self.get(Counter::Miss(Proto::H1)),
            h3_hits: self.get(Counter::Hit(Proto::H3)),
            h3_misses: self.get(Counter::Miss(Proto::H3)),
            evictions_idle: self.get(Counter::EvictionsIdle),
            evictions_lru: self.get(Counter::EvictionsLru),
            evictions_dead: self.get(Counter::EvictionsDead),
            stale_probed: self.get(Counter::StaleProbed),
            h2_ping_failures: self.get(Counter::H2PingFailures),
            installs: self.get(Counter::Installs),
            ..ConnectionStats::default()
        }
    }
}
