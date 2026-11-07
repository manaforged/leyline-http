use crate::wire::{Scheme, TlsInfo};
use foldhash::HashMap;
use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::h2::client::H2Client;
#[cfg(feature = "http3")]
use crate::quic::H3Client;

use crate::pool::h1::H1Io;
use crate::profile::Carrier;

pub(crate) struct Opened<C> {
    pub(crate) conn: C,
    pub(crate) tls: TlsInfo,
    pub(crate) started: Instant,
    pub(crate) connect_ms: Option<u32>,
    pub(crate) lane: u8,
    pub(crate) slot: Option<crate::h2::client::Slot>,
    pub(crate) probe: Option<super::liveness::Probe>,
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
            lane: 0,
            slot: None,
            probe: None,
        }
    }

    pub(crate) fn fresh((conn, tls): (C, TlsInfo), started: Instant) -> Self {
        Self {
            conn,
            tls,
            started,
            connect_ms: Some(crate::wire::ResponseTiming::millis(started)),
            lane: 0,
            slot: None,
            probe: None,
        }
    }

    pub(crate) fn on_lane(mut self, lane: u8) -> Self {
        self.lane = lane;
        self
    }

    pub(crate) fn with_probe(mut self, probe: Option<super::liveness::Probe>) -> Self {
        self.probe = probe;
        self
    }

    pub(crate) fn with_slot(mut self, slot: Option<crate::h2::client::Slot>) -> Self {
        self.slot = slot;
        self
    }

    pub(crate) fn leg(&self) -> LegKind {
        match self.connect_ms {
            Some(_) => LegKind::Fresh,
            None => LegKind::Pooled,
        }
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(crate) struct PoolKey {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) scheme: Scheme,
    pub(crate) proxy: Option<crate::net::ProxyUrl>,
    pub(crate) transport: Carrier,
    pub(crate) partition: Option<u64>,
    pub(crate) lane: u8,
}

impl PoolKey {
    pub(crate) fn host_key(&self) -> Cow<'_, PoolKey> {
        if self.partition.is_none() {
            Cow::Borrowed(self)
        } else {
            Cow::Owned(PoolKey {
                partition: None,
                ..self.clone()
            })
        }
    }

    pub(crate) fn h1_key(self) -> PoolKey {
        PoolKey { lane: 0, ..self }
    }
}

pub struct H1Slot {
    pub(crate) io: Box<dyn H1Io>,
    pub(crate) buf: Vec<u8>,
}

impl H1Slot {
    pub(crate) fn new(io: Box<dyn H1Io>) -> Self {
        Self {
            io,
            buf: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Proto {
    H1,
    H2,
    H3,
}

impl Proto {
    const COUNT: usize = Self::H3 as usize + 1;
}

impl From<http::Version> for Proto {
    fn from(version: http::Version) -> Self {
        match version {
            http::Version::HTTP_2 => Self::H2,
            http::Version::HTTP_3 => Self::H3,
            _ => Self::H1,
        }
    }
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
    const VERSION: http::Version;
    fn into_handle(self) -> MuxHandle;
    fn in_handle(handle: &MuxHandle) -> Option<&Self>;
    fn same(&self, other: &Self) -> bool;
    fn is_closed(&self) -> bool;
}

impl MuxClient for H2Client {
    const VERSION: http::Version = http::Version::HTTP_2;

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

    fn same(&self, other: &Self) -> bool {
        H2Client::same(self, other)
    }

    fn is_closed(&self) -> bool {
        H2Client::is_closed(self)
    }
}

#[cfg(feature = "http3")]
impl MuxClient for H3Client {
    const VERSION: http::Version = http::Version::HTTP_3;

    fn into_handle(self) -> MuxHandle {
        MuxHandle::H3(self)
    }

    fn in_handle(handle: &MuxHandle) -> Option<&Self> {
        match handle {
            MuxHandle::H3(handle) => Some(handle),
            MuxHandle::H2(_) => None,
        }
    }

    fn same(&self, other: &Self) -> bool {
        H3Client::same(self, other)
    }

    fn is_closed(&self) -> bool {
        H3Client::is_closed(self)
    }
}

#[derive(Default)]
pub(crate) struct H1Idle(std::sync::Mutex<VecDeque<(H1Slot, Instant)>>);

impl H1Idle {
    pub(crate) fn queue(&mut self) -> &mut VecDeque<(H1Slot, Instant)> {
        self.0
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn len(&self) -> usize {
        crate::util::lock(&self.0).len()
    }
}

pub(crate) struct Stamp(std::sync::atomic::AtomicU64);

const STAMP_RESOLUTION_NANOS: u64 = 1_000_000;

static EPOCH: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);

impl Stamp {
    pub(crate) fn now() -> Self {
        Self(std::sync::atomic::AtomicU64::new(Self::nanos(
            Instant::now(),
        )))
    }

    pub(crate) fn get(&self) -> Instant {
        *EPOCH + std::time::Duration::from_nanos(self.0.load(std::sync::atomic::Ordering::Relaxed))
    }

    pub(crate) fn touch(&self, now: Instant) -> std::time::Duration {
        let now = Self::nanos(now);
        let before = self.0.load(std::sync::atomic::Ordering::Relaxed);
        let idle = now.saturating_sub(before);
        if idle >= STAMP_RESOLUTION_NANOS {
            self.0.store(now, std::sync::atomic::Ordering::Relaxed);
        }
        std::time::Duration::from_nanos(idle)
    }

    fn nanos(at: Instant) -> u64 {
        u64::try_from(at.saturating_duration_since(*EPOCH).as_nanos()).unwrap_or(u64::MAX)
    }
}

pub(crate) struct MuxEntry {
    pub(crate) handle: MuxHandle,
    pub(crate) last_use: Stamp,
    pub(crate) tls: TlsInfo,
}

impl MuxEntry {
    pub(crate) fn is_dead(&self) -> bool {
        self.handle.is_closed()
    }
}

pub(crate) struct H1Entry {
    pub(crate) idle: H1Idle,
    pub(crate) last_use: Instant,
    pub(crate) tls: TlsInfo,
}

#[derive(Default)]
pub(crate) struct Entries {
    pub(crate) h1: HashMap<PoolKey, H1Entry>,
    pub(crate) mux: HashMap<PoolKey, MuxEntry>,
}

enum Victim {
    H1(PoolKey),
    Mux(PoolKey),
}

impl Entries {
    pub(crate) fn len(&self) -> usize {
        self.h1.len() + self.mux.len()
    }

    pub(crate) fn evict_lru(&mut self, cap: usize) -> u64 {
        let mut evicted = 0u64;
        while self.len() >= cap {
            let h1 = self.h1.iter().min_by_key(|(_, entry)| entry.last_use);
            let mux = self
                .mux
                .iter()
                .map(|(key, entry)| (key, entry.last_use.get()))
                .min_by_key(|(_, last_use)| *last_use);
            let victim = match (h1, mux) {
                (Some((key, entry)), Some((_, last_use))) if entry.last_use <= last_use => {
                    Victim::H1(key.clone())
                }
                (_, Some((key, _))) => Victim::Mux(key.clone()),
                (Some((key, _)), None) => Victim::H1(key.clone()),
                (None, None) => break,
            };
            match victim {
                Victim::H1(key) => drop(self.h1.remove(&key)),
                Victim::Mux(key) => drop(self.mux.remove(&key)),
            }
            evicted += 1;
        }
        evicted
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConnectionStats {
    pub entries: usize,
    pub max_connections: usize,
    pub hits: [u64; Proto::COUNT],
    pub misses: [u64; Proto::COUNT],
    pub evictions_idle: u64,
    pub evictions_lru: u64,
    pub evictions_dead: u64,
    pub stale_probed: u64,
    pub h2_ping_failures: u64,
    pub installs: u64,
    pub busy: usize,
    pub idle: usize,
}

impl ConnectionStats {
    /// Returns how many checkouts on `version` reused a pooled connection.
    pub fn hits(&self, version: http::Version) -> u64 {
        self.hits[Proto::from(version) as usize]
    }

    /// Returns how many checkouts on `version` opened a connection.
    pub fn misses(&self, version: http::Version) -> u64 {
        self.misses[Proto::from(version) as usize]
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Counter {
    EvictionsIdle,
    EvictionsLru,
    EvictionsDead,
    StaleProbed,
    H2PingFailures,
    Installs,
}

impl Counter {
    const COUNT: usize = Self::Installs as usize + 1;
}

#[derive(Default)]
pub(crate) struct PoolCounters {
    hits: [AtomicU64; Proto::COUNT],
    misses: [AtomicU64; Proto::COUNT],
    events: [AtomicU64; Counter::COUNT],
}

impl PoolCounters {
    pub(crate) fn add(&self, counter: Counter, n: u64) {
        self.events[counter as usize].fetch_add(n, Ordering::Relaxed);
    }

    pub(crate) fn checkout(&self, proto: Proto, hit: bool) {
        let counts = if hit { &self.hits } else { &self.misses };
        counts[proto as usize].fetch_add(1, Ordering::Relaxed);
    }

    fn get(&self, counter: Counter) -> u64 {
        self.events[counter as usize].load(Ordering::Relaxed)
    }

    pub(crate) fn snapshot(&self) -> ConnectionStats {
        ConnectionStats {
            hits: self
                .hits
                .each_ref()
                .map(|count| count.load(Ordering::Relaxed)),
            misses: self
                .misses
                .each_ref()
                .map(|count| count.load(Ordering::Relaxed)),
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
