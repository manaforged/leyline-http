//! [`Pool`] struct — thread-safe connection map with LRU eviction.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::h2::client::{DriverTask, H2Client};

use crate::pool::types::{H1Slot, PoolCounters, PoolKey, PoolStats, PooledConn, TlsInfo};

/// Default idle-timeout for pooled connections.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// Default LRU cap. Chosen for a "a few hundred distinct destinations"
/// workload — raise it for workloads with many hosts.
pub const DEFAULT_MAX_CONNECTIONS: usize = 256;

/// HTTP connection pool.
///
/// Thread-safe (`Arc<Mutex<>>`) — a `Session` holds `Arc<Pool>` so
/// cloned sessions share the same pool. Entries are keyed by
/// `(host, port, proxy)` and may be either HTTP/2 (multiplexed
/// clone-handle) or HTTP/1.1 keep-alive (single-checkout owned
/// stream).
pub struct Pool {
    pub(crate) inner: Mutex<HashMap<PoolKey, PooledConn>>,
    pub(crate) idle_timeout: Duration,
    /// LRU cap.
    pub(crate) max_connections: usize,
    pub(crate) counters: PoolCounters,
}

impl Pool {
    /// Create a pool with default 90 s idle timeout and a 256-entry LRU cap.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            counters: PoolCounters::default(),
        }
    }

    /// Create a pool with a custom idle timeout (LRU cap stays default).
    pub fn with_idle_timeout(timeout: Duration) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            idle_timeout: timeout,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            counters: PoolCounters::default(),
        }
    }

    /// Create a pool with explicit idle timeout and LRU cap.
    pub fn with_limits(idle_timeout: Duration, max_connections: usize) -> Self {
        assert!(max_connections > 0, "max_connections must be at least 1");
        Self {
            inner: Mutex::new(HashMap::new()),
            idle_timeout,
            max_connections,
            counters: PoolCounters::default(),
        }
    }

    /// Observability snapshot.
    pub fn stats(&self) -> PoolStats {
        let entries = self.inner.lock().unwrap_or_else(|e| e.into_inner()).len();
        PoolStats {
            entries,
            max_connections: self.max_connections,
            h2_hits: self.counters.h2_hits.load(Ordering::Relaxed),
            h2_misses: self.counters.h2_misses.load(Ordering::Relaxed),
            h1_hits: self.counters.h1_hits.load(Ordering::Relaxed),
            h1_misses: self.counters.h1_misses.load(Ordering::Relaxed),
            evictions_idle: self.counters.evictions_idle.load(Ordering::Relaxed),
            evictions_lru: self.counters.evictions_lru.load(Ordering::Relaxed),
            evictions_dead: self.counters.evictions_dead.load(Ordering::Relaxed),
            installs: self.counters.installs.load(Ordering::Relaxed),
        }
    }

    /// Evict idle connections AND any entry whose underlying handle is
    /// already dead. Called before every checkout.
    pub(crate) fn evict_idle(&self) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let before = map.len();
        map.retain(|_, entry| {
            !entry.is_dead() && now.duration_since(entry.last_use()) < self.idle_timeout
        });
        let evicted = (before - map.len()) as u64;
        if evicted > 0 {
            self.counters
                .evictions_idle
                .fetch_add(evicted, Ordering::Relaxed);
        }
    }

    /// Drop the LRU entry when at or over cap. Returns eviction count.
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

    /// Look up a live H2 handle for `key`, touching its last-use timestamp.
    pub(crate) fn checkout_h2(&self, key: &PoolKey) -> Option<(H2Client, TlsInfo)> {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let dead = map.get(key).is_some_and(PooledConn::is_dead);
        if dead {
            map.remove(key);
            self.counters.evictions_dead.fetch_add(1, Ordering::Relaxed);
            self.counters.h2_misses.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        match map.get_mut(key) {
            Some(entry @ PooledConn::H2 { .. }) => {
                entry.set_last_use(Instant::now());
                if let PooledConn::H2 { handle, tls, .. } = entry {
                    let out = (handle.clone(), tls.clone());
                    self.counters.h2_hits.fetch_add(1, Ordering::Relaxed);
                    Some(out)
                } else {
                    unreachable!()
                }
            }
            _ => {
                self.counters.h2_misses.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    /// Look up and take a live H1 keep-alive slot for `key`.
    pub(crate) fn checkout_h1(&self, key: &PoolKey) -> Option<(H1Slot, TlsInfo)> {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let is_dead = map.get(key).is_some_and(PooledConn::is_dead);
        if is_dead {
            map.remove(key);
            self.counters.evictions_dead.fetch_add(1, Ordering::Relaxed);
            self.counters.h1_misses.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        match map.get_mut(key) {
            Some(PooledConn::H1 {
                conn,
                last_use,
                tls,
            }) => {
                let slot = conn
                    .get_mut()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                match slot {
                    Some(slot) => {
                        *last_use = Instant::now();
                        let tls = tls.clone();
                        self.counters.h1_hits.fetch_add(1, Ordering::Relaxed);
                        Some((slot, tls))
                    }
                    None => {
                        self.counters.h1_misses.fetch_add(1, Ordering::Relaxed);
                        None
                    }
                }
            }
            _ => {
                self.counters.h1_misses.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    /// Install a new H2 pooled connection.
    pub(crate) fn install_h2(
        &self,
        key: PoolKey,
        handle: H2Client,
        driver: DriverTask,
        tls: TlsInfo,
    ) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if !map.contains_key(&key) {
            let evicted = Self::evict_lru_if_needed(&mut map, self.max_connections);
            if evicted > 0 {
                self.counters
                    .evictions_lru
                    .fetch_add(evicted, Ordering::Relaxed);
            }
        }
        map.insert(
            key,
            PooledConn::H2 {
                handle,
                _driver: Some(driver),
                last_use: Instant::now(),
                tls,
            },
        );
        self.counters.installs.fetch_add(1, Ordering::Relaxed);
    }

    /// Install a new H1 keep-alive slot under `key`.
    pub(crate) fn install_h1(&self, key: PoolKey, slot: H1Slot, tls: TlsInfo, count_install: bool) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if !map.contains_key(&key) {
            let evicted = Self::evict_lru_if_needed(&mut map, self.max_connections);
            if evicted > 0 {
                self.counters
                    .evictions_lru
                    .fetch_add(evicted, Ordering::Relaxed);
            }
        }
        map.insert(
            key,
            PooledConn::H1 {
                conn: Mutex::new(Some(slot)),
                last_use: Instant::now(),
                tls,
            },
        );
        if count_install {
            self.counters.installs.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Current entry count.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// True when the pool has no entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Invalidate the entry for `key` after a connection-level error.
    pub(crate) fn invalidate(&self, key: &PoolKey) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if map.remove(key).is_some() {
            self.counters.evictions_dead.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl Default for Pool {
    fn default() -> Self {
        Self::new()
    }
}
