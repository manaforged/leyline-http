use foldhash::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};

#[cfg(feature = "http3")]
use self::alt_svc::AltSvcCache;
use crate::pool::connect::{H2Dial, Inflight};
use crate::util::expiring::ExpiringSet;
use tokio::sync::Semaphore;

use super::config::{Io, PoolConfig};
#[cfg(feature = "bench-internals")]
use crate::h2::client::H2Client;
#[cfg(feature = "http3")]
use crate::quic::H3Client;

use crate::pool::ConnectionStats;
#[cfg(feature = "bench-internals")]
use crate::pool::types::MuxHandle;
use crate::pool::types::{Counter, Entries, MuxClient, PoolCounters, PoolKey, Proto};
use crate::profile::Carrier;
use crate::util::{lock, write};
use crate::wire::Scheme;

pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

pub const DEFAULT_MAX_CONNECTIONS: usize = 2048;

pub const DEFAULT_MAX_H1_CONNS_PER_HOST: usize = 256;

const POOL_REAP_INTERVAL: Duration = Duration::from_millis(250);

const H1_ONLY_TTL: Duration = Duration::from_secs(600);
const H3_BROKEN_TTL: Duration = Duration::from_secs(300);

const H2_SPILL_PAUSE: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy)]
pub(crate) enum OriginFlag {
    H1Only,
    #[cfg_attr(
        not(feature = "http3"),
        allow(dead_code, reason = "HTTP/3 failures exist only with http3")
    )]
    H3Broken,
    H2SpillFailed,
}

impl OriginFlag {
    const COUNT: usize = Self::H2SpillFailed as usize + 1;

    pub(crate) const fn ttl(self) -> Duration {
        match self {
            Self::H1Only => H1_ONLY_TTL,
            Self::H3Broken => H3_BROKEN_TTL,
            Self::H2SpillFailed => H2_SPILL_PAUSE,
        }
    }
}

type OriginProxyKey = (String, u16, Option<String>);

pub(crate) type OriginMemory = [ExpiringSet<OriginProxyKey>; OriginFlag::COUNT];

struct Conns {
    entries: RwLock<Entries>,
    inflight_h2: Inflight<H2Dial>,
    #[cfg(feature = "http3")]
    inflight_h3: Inflight<H3Client>,
    h1_permits: Mutex<HashMap<PoolKey, Arc<Semaphore>>>,
    counters: PoolCounters,
    created: Instant,
    next_reap_ms: AtomicU64,
}

impl Default for Conns {
    fn default() -> Self {
        Self {
            entries: RwLock::default(),
            inflight_h2: Inflight::default(),
            #[cfg(feature = "http3")]
            inflight_h3: Inflight::default(),
            h1_permits: Mutex::default(),
            counters: PoolCounters::default(),
            created: Instant::now(),
            next_reap_ms: AtomicU64::default(),
        }
    }
}

#[derive(Default)]
struct Memory {
    #[cfg(feature = "http3")]
    alt_svc: Mutex<AltSvcCache>,
    origins: Mutex<OriginMemory>,
}

#[derive(Clone)]
pub struct Pool {
    conns: Arc<Conns>,
    memory: Arc<Memory>,
    config: PoolConfig,
    io: Io,
    h2_lanes: u8,
    partition: Option<u64>,
}

impl Pool {
    pub(crate) fn config(&self) -> &PoolConfig {
        &self.config
    }

    pub(crate) fn io(&self) -> Io {
        self.io
    }

    pub(crate) fn h2_lanes(&self) -> u8 {
        self.h2_lanes
    }

    pub(crate) fn drain_wait(&self) -> Duration {
        if self.config.keepalive {
            self.config.idle_timeout
        } else {
            Duration::ZERO
        }
    }

    pub(crate) fn inflight_h2(&self) -> &Inflight<H2Dial> {
        &self.conns.inflight_h2
    }

    #[cfg(feature = "http3")]
    pub(crate) fn inflight_h3(&self) -> &Inflight<H3Client> {
        &self.conns.inflight_h3
    }

    pub(crate) fn note_checkout(&self, proto: Proto, hit: bool) {
        self.conns.counters.checkout(proto, hit);
    }

    pub(crate) fn note(&self, counter: Counter) {
        self.conns.counters.add(counter, 1);
    }

    pub(crate) fn remember(
        &self,
        flag: OriginFlag,
        host: &str,
        port: u16,
        proxy: Option<&crate::net::ProxyUrl>,
    ) {
        let now = SystemTime::now();
        lock(&self.memory.origins)[flag as usize].insert(
            origin_proxy_key(host, port, proxy),
            now + flag.ttl(),
            now,
        );
    }

    pub(crate) fn recalls(
        &self,
        flag: OriginFlag,
        host: &str,
        port: u16,
        proxy: Option<&crate::net::ProxyUrl>,
    ) -> bool {
        lock(&self.memory.origins)[flag as usize]
            .contains(&origin_proxy_key(host, port, proxy), SystemTime::now())
    }

    pub(super) fn forget(
        &self,
        flag: OriginFlag,
        host: &str,
        port: u16,
        proxy: Option<&crate::net::ProxyUrl>,
    ) {
        lock(&self.memory.origins)[flag as usize].remove(&origin_proxy_key(host, port, proxy));
    }

    #[cfg(feature = "http3")]
    pub(crate) fn note_alt_svc(&self, host: &str, port: u16, fields: &[&str], age: Duration) {
        lock(&self.memory.alt_svc).note(host, port, fields, age, std::time::SystemTime::now());
    }

    #[cfg(feature = "http3")]
    pub(crate) fn knows_h3(&self, host: &str, port: u16) -> bool {
        lock(&self.memory.alt_svc).knows_h3(host, port, std::time::SystemTime::now())
    }

    #[cfg(feature = "http3")]
    pub(crate) fn alt_svc_entries(&self) -> Vec<(String, u16, SystemTime)> {
        lock(&self.memory.alt_svc).export(SystemTime::now())
    }

    #[cfg(feature = "http3")]
    pub(crate) fn restore_alt_svc(&self, entries: &[(String, u16, SystemTime)]) {
        lock(&self.memory.alt_svc).import(entries, SystemTime::now());
    }

    #[cfg(any(test, feature = "bench-internals"))]
    pub fn new() -> Self {
        Self::with_limits(
            DEFAULT_IDLE_TIMEOUT,
            DEFAULT_MAX_CONNECTIONS,
            DEFAULT_MAX_H1_CONNS_PER_HOST,
        )
    }

    #[cfg(any(test, feature = "bench-internals"))]
    pub fn with_limits(
        idle_timeout: Duration,
        max_connections: usize,
        max_h1_conns_per_host: usize,
    ) -> Self {
        Self::from_config(
            PoolConfig {
                idle_timeout,
                max_connections,
                max_h1_conns_per_host,
                max_h2_conns_per_host: Some(1),
                ..PoolConfig::default()
            },
            Io::Async,
            false,
        )
    }

    pub(crate) fn from_config(config: PoolConfig, io: Io, impersonates: bool) -> Self {
        assert!(
            config.max_connections > 0,
            "max_connections must be at least 1"
        );
        assert!(
            config.max_h1_conns_per_host > 0,
            "max_h1_conns_per_host must be at least 1"
        );
        Self {
            conns: Arc::default(),
            memory: Arc::default(),
            config,
            io,
            h2_lanes: u8::try_from(config.h2_conns_per_host(impersonates)).unwrap_or(u8::MAX),
            partition: None,
        }
    }

    pub(crate) fn shared_view(&self, partition: Option<u64>) -> Self {
        Self {
            partition,
            ..self.clone()
        }
    }

    pub(crate) fn sibling(&self) -> Self {
        Self {
            conns: Arc::default(),
            ..self.clone()
        }
    }

    pub(crate) fn key(
        &self,
        scheme: Scheme,
        host: &str,
        port: u16,
        proxy: Option<&crate::net::ProxyUrl>,
        transport: Carrier,
    ) -> PoolKey {
        PoolKey {
            scheme,
            host: host.to_string(),
            port,
            proxy: proxy.cloned(),
            transport,
            partition: self.partition,
            lane: 0,
        }
    }

    pub fn stats(&self) -> ConnectionStats {
        let entries = write(&self.conns.entries).len();
        let counts = self.connection_counts();
        ConnectionStats {
            entries,
            busy: counts.busy,
            idle: counts.idle,
            max_connections: self.config.max_connections,
            ..self.conns.counters.snapshot()
        }
    }

    pub(crate) fn evict_idle(&self, now: Instant) {
        let conns = &*self.conns;
        let now_ms = now.duration_since(conns.created).as_millis() as u64;
        if now_ms < conns.next_reap_ms.load(Ordering::Relaxed) {
            return;
        }
        conns.next_reap_ms.store(
            now_ms + POOL_REAP_INTERVAL.as_millis() as u64,
            Ordering::Relaxed,
        );
        let idle_timeout = self.config.idle_timeout;
        let mut idle_evicted = 0u64;
        {
            let mut entries = write(&conns.entries);
            entries.h1.retain(|_, entry| {
                let idle = entry.idle.queue();
                let before = idle.len();
                idle.retain(|(_, returned_at)| now.duration_since(*returned_at) < idle_timeout);
                idle_evicted += (before - idle.len()) as u64;
                !idle.is_empty() || now.duration_since(entry.last_use) < idle_timeout
            });
            entries.mux.retain(|_, entry| {
                let keep =
                    !entry.is_dead() && now.duration_since(entry.last_use.get()) < idle_timeout;
                if !keep {
                    idle_evicted += 1;
                }
                keep
            });
        }
        {
            let mut permits = lock(&conns.h1_permits);
            permits.retain(|_, sem| Arc::strong_count(sem) > 1);
        }
        if idle_evicted > 0 {
            conns.counters.add(Counter::EvictionsIdle, idle_evicted);
        }
    }

    pub(crate) fn stale_hit(
        &self,
        key: &PoolKey,
        version: http::Version,
        error: &dyn std::fmt::Display,
    ) {
        tracing::info!(
            target: "leyline::pool",
            host = %key.host,
            port = key.port,
            proxied = key.proxy.is_some(),
            version = ?version,
            error = %error,
            "pool stale hit -- pooled connection failed, opening fresh"
        );
    }

    /// Drops the entry at `key` when it still holds `failed`, so a replacement stays pooled.
    pub(crate) fn invalidate<C: MuxClient>(&self, key: &PoolKey, failed: &C) {
        let mut entries = write(&self.conns.entries);
        let holds = entries
            .mux
            .get(key)
            .and_then(|entry| C::in_handle(&entry.handle))
            .is_some_and(|c| c.same(failed));
        if holds {
            entries.mux.remove(key);
            self.conns.counters.add(Counter::EvictionsDead, 1);
        }
    }

    #[cfg(feature = "bench-internals")]
    pub fn bench_populate_h2(&self, n: usize, handle: &H2Client) {
        let mut entries = write(&self.conns.entries);
        entries.h1.clear();
        entries.mux.clear();
        for i in 0..n {
            entries.mux.insert(
                self.key(
                    Scheme::Https,
                    &format!("h{i}.bench"),
                    443,
                    None,
                    Carrier::Tcp,
                ),
                super::types::MuxEntry {
                    handle: MuxHandle::H2(handle.clone()),
                    last_use: super::types::Stamp::now(),
                    tls: crate::wire::TlsInfo::default(),
                },
            );
        }
    }

    #[cfg(feature = "bench-internals")]
    pub fn bench_probe(&self) -> bool {
        let key = self.key(Scheme::Https, "h0.bench", 443, None, Carrier::Tcp);
        self.evict_idle(Instant::now());
        self.checkout_mux::<H2Client>(&key).is_some()
    }
}

fn origin_proxy_key(host: &str, port: u16, proxy: Option<&crate::net::ProxyUrl>) -> OriginProxyKey {
    (host.to_string(), port, proxy.map(|p| p.as_str().to_owned()))
}

#[cfg(any(test, feature = "bench-internals"))]
impl Default for Pool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;

mod counts;
mod slots;

#[cfg(feature = "http3")]
mod alt_svc;
