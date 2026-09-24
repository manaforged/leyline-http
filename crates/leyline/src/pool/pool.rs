use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::pool::connect::Inflight;
use tokio::sync::Semaphore;

use crate::h2::client::H2Client;
#[cfg(feature = "http3")]
use crate::quic::H3Client;

use crate::pool::types::{PoolCounters, PoolKey, PoolStats, PooledConn};
#[cfg(feature = "bench-internals")]
use crate::pool::types::{TlsInfo, Transport};

pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

pub const DEFAULT_MAX_CONNECTIONS: usize = 2048;

pub const DEFAULT_MAX_H1_CONNS_PER_HOST: usize = 256;

const POOL_REAP_INTERVAL: Duration = Duration::from_millis(250);

pub struct Pool {
    pub(crate) inner: Mutex<HashMap<PoolKey, PooledConn>>,
    pub(crate) inflight_h2: Inflight<H2Client>,
    #[cfg(feature = "http3")]
    pub(crate) inflight_h3: Inflight<H3Client>,
    #[cfg(feature = "http3")]
    pub(crate) h3_known: Mutex<HashSet<(String, u16)>>,
    pub(crate) h1_only: Mutex<HashSet<(String, u16, Option<String>)>>,
    pub(crate) idle_timeout: Duration,
    pub(crate) max_connections: usize,
    pub(crate) max_h1_conns_per_host: usize,
    pub(crate) h2_ping_after_idle: Option<Duration>,
    pub(crate) h2_ping_timeout: Duration,
    pub(crate) h1_permits: Mutex<HashMap<PoolKey, Arc<Semaphore>>>,
    pub(crate) counters: PoolCounters,
    created: Instant,
    next_reap_ms: AtomicU64,
}

impl Pool {
    pub(crate) fn note_h1_only(&self, host: &str, port: u16, proxy: Option<&str>) {
        self.h1_only
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert((host.to_string(), port, proxy.map(str::to_string)));
    }

    pub(crate) fn is_h1_only(&self, host: &str, port: u16, proxy: Option<&str>) -> bool {
        self.h1_only
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&(host.to_string(), port, proxy.map(str::to_string)))
    }

    #[cfg(feature = "http3")]
    pub(crate) fn note_h3(&self, host: &str, port: u16) {
        self.h3_known
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert((host.to_string(), port));
    }

    #[cfg(feature = "http3")]
    pub(crate) fn note_alt_svc(&self, host: &str, port: u16, alt_svc: &str) {
        if alt_svc
            .split(',')
            .any(|alt| alt_svc_same_authority(alt.trim_start(), host, port))
        {
            self.note_h3(host, port);
        }
    }

    #[cfg(feature = "http3")]
    pub(crate) fn knows_h3(&self, host: &str, port: u16) -> bool {
        self.h3_known
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&(host.to_string(), port))
    }

    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            inflight_h2: Inflight::default(),
            #[cfg(feature = "http3")]
            inflight_h3: Inflight::default(),
            #[cfg(feature = "http3")]
            h3_known: Mutex::new(HashSet::new()),
            h1_only: Mutex::new(HashSet::new()),
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            max_h1_conns_per_host: DEFAULT_MAX_H1_CONNS_PER_HOST,
            h2_ping_after_idle: super::liveness::DEFAULT_H2_PING_AFTER_IDLE,
            h2_ping_timeout: super::liveness::DEFAULT_H2_PING_TIMEOUT,
            h1_permits: Mutex::new(HashMap::new()),
            counters: PoolCounters::default(),
            created: Instant::now(),
            next_reap_ms: AtomicU64::new(0),
        }
    }

    pub fn with_limits(
        idle_timeout: Duration,
        max_connections: usize,
        max_h1_conns_per_host: usize,
    ) -> Self {
        assert!(max_connections > 0, "max_connections must be at least 1");
        assert!(
            max_h1_conns_per_host > 0,
            "max_h1_conns_per_host must be at least 1"
        );
        Self {
            inner: Mutex::new(HashMap::new()),
            inflight_h2: Inflight::default(),
            #[cfg(feature = "http3")]
            inflight_h3: Inflight::default(),
            #[cfg(feature = "http3")]
            h3_known: Mutex::new(HashSet::new()),
            h1_only: Mutex::new(HashSet::new()),
            idle_timeout,
            max_connections,
            max_h1_conns_per_host,
            h2_ping_after_idle: super::liveness::DEFAULT_H2_PING_AFTER_IDLE,
            h2_ping_timeout: super::liveness::DEFAULT_H2_PING_TIMEOUT,
            h1_permits: Mutex::new(HashMap::new()),
            counters: PoolCounters::default(),
            created: Instant::now(),
            next_reap_ms: AtomicU64::new(0),
        }
    }

    pub fn stats(&self) -> PoolStats {
        let entries = self.inner.lock().unwrap_or_else(|e| e.into_inner()).len();
        PoolStats {
            entries,
            max_connections: self.max_connections,
            h2_hits: self.counters.h2_hits.load(Ordering::Relaxed),
            h2_misses: self.counters.h2_misses.load(Ordering::Relaxed),
            h1_hits: self.counters.h1_hits.load(Ordering::Relaxed),
            h1_misses: self.counters.h1_misses.load(Ordering::Relaxed),
            h3_hits: self.counters.h3_hits.load(Ordering::Relaxed),
            h3_misses: self.counters.h3_misses.load(Ordering::Relaxed),
            evictions_idle: self.counters.evictions_idle.load(Ordering::Relaxed),
            evictions_lru: self.counters.evictions_lru.load(Ordering::Relaxed),
            evictions_dead: self.counters.evictions_dead.load(Ordering::Relaxed),
            stale_probed: self.counters.stale_probed.load(Ordering::Relaxed),
            h2_ping_failures: self.counters.h2_ping_failures.load(Ordering::Relaxed),
            installs: self.counters.installs.load(Ordering::Relaxed),
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
        let mut idle_evicted = 0u64;
        {
            let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            map.retain(|_, entry| match entry {
                PooledConn::H1 { idle, last_use, .. } => {
                    let before = idle.len();
                    idle.retain(|(_, returned_at)| {
                        now.duration_since(*returned_at) < self.idle_timeout
                    });
                    idle_evicted += (before - idle.len()) as u64;
                    !idle.is_empty() || now.duration_since(*last_use) < self.idle_timeout
                }
                PooledConn::H2 {
                    handle, last_use, ..
                } => {
                    let keep =
                        !handle.is_closed() && now.duration_since(*last_use) < self.idle_timeout;
                    if !keep {
                        idle_evicted += 1;
                    }
                    keep
                }
                #[cfg(feature = "http3")]
                PooledConn::H3 {
                    handle, last_use, ..
                } => {
                    let keep =
                        !handle.is_closed() && now.duration_since(*last_use) < self.idle_timeout;
                    if !keep {
                        idle_evicted += 1;
                    }
                    keep
                }
            });
        }
        {
            let mut permits = self.h1_permits.lock().unwrap_or_else(|e| e.into_inner());
            permits.retain(|_, sem| Arc::strong_count(sem) > 1);
        }
        if idle_evicted > 0 {
            self.counters
                .evictions_idle
                .fetch_add(idle_evicted, Ordering::Relaxed);
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
        self.counters.installs.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn note_h1_dead(&self) {
        self.counters.evictions_dead.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn note_h1_stale_probed(&self) {
        self.counters.stale_probed.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn fresh(&self) -> Self {
        Self::with_limits(
            self.idle_timeout,
            self.max_connections,
            self.max_h1_conns_per_host,
        )
        .with_h2_ping(self.h2_ping_after_idle, self.h2_ping_timeout)
    }

    pub(crate) fn invalidate(&self, key: &PoolKey) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if map.remove(key).is_some() {
            self.counters.evictions_dead.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[cfg(feature = "bench-internals")]
    pub fn bench_populate_h2(&self, n: usize, handle: &H2Client) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        map.clear();
        for i in 0..n {
            map.insert(
                PoolKey {
                    host: format!("h{i}.bench"),
                    scheme: "https".to_string(),
                    port: 443,
                    proxy: None,
                    transport: Transport::Tcp,
                },
                PooledConn::H2 {
                    handle: handle.clone(),
                    last_use: Instant::now(),
                    tls: TlsInfo::default(),
                },
            );
        }
    }

    #[cfg(feature = "bench-internals")]
    pub fn bench_probe(&self) -> bool {
        let key = PoolKey {
            host: "h0.bench".to_string(),
            scheme: "https".to_string(),
            port: 443,
            proxy: None,
            transport: Transport::Tcp,
        };
        self.evict_idle();
        self.checkout_h2(&key).is_some()
    }
}

impl Default for Pool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "http3")]
fn alt_svc_same_authority(entry: &str, host: &str, port: u16) -> bool {
    let Some(value) = entry.strip_prefix("h3=") else {
        return false;
    };
    let authority = value.split(';').next().unwrap_or_default().trim();
    let authority = authority
        .strip_prefix('"')
        .and_then(|a| a.strip_suffix('"'))
        .unwrap_or(authority);
    match authority.rsplit_once(':') {
        Some((alt_host, alt_port)) => {
            alt_port.parse::<u16>() == Ok(port)
                && (alt_host.is_empty() || alt_host.eq_ignore_ascii_case(host))
        }
        None => false,
    }
}

#[cfg(test)]
mod tests;

mod slots;
