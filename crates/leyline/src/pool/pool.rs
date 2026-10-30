use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

#[cfg(feature = "http3")]
use self::alt_svc::AltSvcCache;
use self::expiring::ExpiringSet;
use crate::pool::connect::Inflight;
use tokio::sync::Semaphore;

use crate::core::PoolConfig;
use crate::h2::client::H2Client;
#[cfg(feature = "http3")]
use crate::quic::H3Client;

use crate::pool::types::{ConnectionStats, Counter, PoolCounters, PoolKey, PooledConn, Transport};
#[cfg(feature = "bench-internals")]
use crate::pool::types::{MuxHandle, TlsInfo};
use crate::util::lock;

pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

pub const DEFAULT_MAX_CONNECTIONS: usize = 2048;

pub const DEFAULT_MAX_H1_CONNS_PER_HOST: usize = 256;

const POOL_REAP_INTERVAL: Duration = Duration::from_millis(250);

const H1_ONLY_TTL: Duration = Duration::from_secs(600);
const H3_BROKEN_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy)]
pub(crate) enum OriginFlag {
    H1Only,
    #[cfg_attr(
        not(feature = "http3"),
        allow(dead_code, reason = "HTTP/3 failures exist only with http3")
    )]
    H3Broken,
}

impl OriginFlag {
    const COUNT: usize = 2;

    pub(crate) const fn ttl(self) -> Duration {
        match self {
            Self::H1Only => H1_ONLY_TTL,
            Self::H3Broken => H3_BROKEN_TTL,
        }
    }
}

type OriginProxyKey = (String, u16, Option<String>);

pub(crate) type OriginMemory = [ExpiringSet<OriginProxyKey>; OriginFlag::COUNT];

pub struct Pool {
    pub(crate) inner: Arc<Mutex<HashMap<PoolKey, PooledConn>>>,
    pub(crate) inflight_h2: Inflight<H2Client>,
    #[cfg(feature = "http3")]
    pub(crate) inflight_h3: Inflight<H3Client>,
    #[cfg(feature = "http3")]
    pub(crate) alt_svc: Arc<Mutex<AltSvcCache>>,
    pub(crate) origin_memory: Arc<Mutex<OriginMemory>>,
    pub(crate) hsts: Arc<Mutex<crate::core::hsts::HstsStore>>,
    pub(crate) config: PoolConfig,
    pub(crate) max_body_size: usize,
    pub(crate) h1_permits: Arc<Mutex<HashMap<PoolKey, Arc<Semaphore>>>>,
    pub(crate) counters: Arc<PoolCounters>,
    pub(crate) partition: u64,
    created: Instant,
    next_reap_ms: Arc<AtomicU64>,
}

impl Pool {
    pub(crate) fn remember(&self, flag: OriginFlag, host: &str, port: u16, proxy: Option<&str>) {
        let now = SystemTime::now();
        lock(&self.origin_memory)[flag as usize].insert(
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
        proxy: Option<&str>,
    ) -> bool {
        lock(&self.origin_memory)[flag as usize]
            .contains(&origin_proxy_key(host, port, proxy), SystemTime::now())
    }

    pub(crate) fn forget(&self, flag: OriginFlag, host: &str, port: u16, proxy: Option<&str>) {
        lock(&self.origin_memory)[flag as usize].remove(&origin_proxy_key(host, port, proxy));
    }

    #[cfg(feature = "http3")]
    pub(crate) fn note_alt_svc(&self, host: &str, port: u16, fields: &[&str], age: Duration) {
        lock(&self.alt_svc).note(host, port, fields, age, std::time::SystemTime::now());
    }

    #[cfg(feature = "http3")]
    pub(crate) fn knows_h3(&self, host: &str, port: u16) -> bool {
        lock(&self.alt_svc).knows_h3(host, port, std::time::SystemTime::now())
    }

    #[cfg(feature = "http3")]
    pub(crate) fn alt_svc_entries(&self) -> Vec<(String, u16, SystemTime)> {
        lock(&self.alt_svc).export(SystemTime::now())
    }

    #[cfg(feature = "http3")]
    pub(crate) fn restore_alt_svc(&self, entries: &[(String, u16, SystemTime)]) {
        lock(&self.alt_svc).import(entries, SystemTime::now());
    }

    pub fn new() -> Self {
        Self::with_limits(
            DEFAULT_IDLE_TIMEOUT,
            DEFAULT_MAX_CONNECTIONS,
            DEFAULT_MAX_H1_CONNS_PER_HOST,
        )
    }

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
                ..PoolConfig::default()
            },
            crate::core::DEFAULT_MAX_BODY_SIZE,
        )
    }

    pub(crate) fn from_config(config: PoolConfig, max_body_size: usize) -> Self {
        assert!(
            config.max_connections > 0,
            "max_connections must be at least 1"
        );
        assert!(
            config.max_h1_conns_per_host > 0,
            "max_h1_conns_per_host must be at least 1"
        );
        Self {
            inner: Arc::default(),
            inflight_h2: Inflight::default(),
            #[cfg(feature = "http3")]
            inflight_h3: Inflight::default(),
            #[cfg(feature = "http3")]
            alt_svc: Arc::default(),
            origin_memory: Arc::default(),
            hsts: Arc::default(),
            config,
            max_body_size,
            h1_permits: Arc::default(),
            counters: Arc::default(),
            partition: 0,
            created: Instant::now(),
            next_reap_ms: Arc::default(),
        }
    }

    pub(crate) fn shared_view(&self, partition: u64) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            inflight_h2: self.inflight_h2.clone(),
            #[cfg(feature = "http3")]
            inflight_h3: self.inflight_h3.clone(),
            #[cfg(feature = "http3")]
            alt_svc: Arc::clone(&self.alt_svc),
            origin_memory: Arc::clone(&self.origin_memory),
            hsts: Arc::clone(&self.hsts),
            config: self.config,
            max_body_size: self.max_body_size,
            h1_permits: Arc::clone(&self.h1_permits),
            counters: Arc::clone(&self.counters),
            partition,
            created: self.created,
            next_reap_ms: Arc::clone(&self.next_reap_ms),
        }
    }

    pub(crate) fn key(
        &self,
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
            partition: self.partition,
        }
    }

    pub fn stats(&self) -> ConnectionStats {
        let entries = lock(&self.inner).len();
        let counts = self.connection_counts();
        ConnectionStats {
            entries,
            busy: counts.busy,
            idle: counts.idle,
            max_connections: self.config.max_connections,
            ..self.counters.snapshot()
        }
    }

    pub(crate) fn evict_idle(&self) {
        let now = Instant::now();
        let now_ms = now.duration_since(self.created).as_millis() as u64;
        if now_ms < self.next_reap_ms.load(Ordering::Relaxed) {
            return;
        }
        self.next_reap_ms.store(
            now_ms + POOL_REAP_INTERVAL.as_millis() as u64,
            Ordering::Relaxed,
        );
        let idle_timeout = self.config.idle_timeout;
        let mut idle_evicted = 0u64;
        {
            let mut map = lock(&self.inner);
            map.retain(|_, entry| match entry {
                PooledConn::H1 { idle, last_use, .. } => {
                    let before = idle.len();
                    idle.retain(|(_, returned_at)| now.duration_since(*returned_at) < idle_timeout);
                    idle_evicted += (before - idle.len()) as u64;
                    !idle.is_empty() || now.duration_since(*last_use) < idle_timeout
                }
                PooledConn::Mux {
                    handle, last_use, ..
                } => {
                    let keep = !handle.is_closed() && now.duration_since(*last_use) < idle_timeout;
                    if !keep {
                        idle_evicted += 1;
                    }
                    keep
                }
            });
        }
        {
            let mut permits = lock(&self.h1_permits);
            permits.retain(|_, sem| Arc::strong_count(sem) > 1);
        }
        if idle_evicted > 0 {
            self.counters.add(Counter::EvictionsIdle, idle_evicted);
        }
    }

    pub(crate) fn evict_lru_if_needed(map: &mut HashMap<PoolKey, PooledConn>, cap: usize) -> u64 {
        let mut evicted = 0u64;
        while map.len() >= cap {
            let victim = map
                .iter()
                .min_by_key(|(_, e)| e.last_use())
                .map(|(k, _)| k.clone());
            match victim {
                Some(k) => {
                    map.remove(&k);
                    evicted += 1;
                }
                None => break,
            }
        }
        evicted
    }

    pub(crate) fn note_h1_install(&self) {
        self.counters.add(Counter::Installs, 1);
    }

    pub(crate) fn note_h1_dead(&self) {
        self.counters.add(Counter::EvictionsDead, 1);
    }

    pub(crate) fn note_h1_stale_probed(&self) {
        self.counters.add(Counter::StaleProbed, 1);
    }

    pub(crate) fn invalidate(&self, key: &PoolKey) {
        let mut map = lock(&self.inner);
        if map.remove(key).is_some() {
            self.counters.add(Counter::EvictionsDead, 1);
        }
    }

    #[cfg(feature = "bench-internals")]
    pub fn bench_populate_h2(&self, n: usize, handle: &H2Client) {
        let mut map = lock(&self.inner);
        map.clear();
        for i in 0..n {
            map.insert(
                self.key("https", &format!("h{i}.bench"), 443, None, Transport::Tcp),
                PooledConn::Mux {
                    handle: MuxHandle::H2(handle.clone()),
                    last_use: Instant::now(),
                    tls: TlsInfo::default(),
                },
            );
        }
    }

    #[cfg(feature = "bench-internals")]
    pub fn bench_probe(&self) -> bool {
        let key = self.key("https", "h0.bench", 443, None, Transport::Tcp);
        self.evict_idle();
        self.checkout_mux::<H2Client>(&key).is_some()
    }
}

fn origin_proxy_key(host: &str, port: u16, proxy: Option<&str>) -> OriginProxyKey {
    (host.to_string(), port, proxy.map(str::to_string))
}

impl Default for Pool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;

mod counts;
mod slots;

pub(crate) mod expiring;

#[cfg(feature = "http3")]
mod alt_svc;
