//! [`Pool`] struct — thread-safe connection map with LRU eviction.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::future::{BoxFuture, Shared};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::h2::client::{DriverTask, H2Client};
#[cfg(feature = "http3")]
use crate::quic::{H3Client, H3DriverTask};

#[cfg(feature = "bench-internals")]
use crate::pool::types::Transport;
use crate::pool::types::{H1Slot, PoolCounters, PoolKey, PoolStats, PooledConn, TlsInfo};

/// A shared, awaitable in-progress H2 connect.
pub(crate) type SharedConnect =
    Shared<BoxFuture<'static, Result<(H2Client, TlsInfo), Arc<crate::Error>>>>;

/// Shared in-flight QUIC + HTTP/3 connect.
#[cfg(feature = "http3")]
pub(crate) type SharedH3Connect =
    Shared<BoxFuture<'static, Result<(H3Client, TlsInfo), Arc<crate::Error>>>>;

/// Default idle-timeout for pooled connections.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// Default LRU cap: 2048 entries.
pub const DEFAULT_MAX_CONNECTIONS: usize = 2048;

/// Default cap on simultaneous HTTP/1.1 connections **per destination** `(host, port, proxy)`.
pub const DEFAULT_MAX_H1_CONNS_PER_HOST: usize = 256;

/// Minimum spacing between full idle/permit sweeps.
const POOL_REAP_INTERVAL: Duration = Duration::from_millis(250);

/// HTTP connection pool.
pub struct Pool {
    pub(crate) inner: Mutex<HashMap<PoolKey, PooledConn>>,
    /// In-progress H2 connects, keyed like `inner`.
    pub(crate) inflight_h2: Mutex<HashMap<PoolKey, SharedConnect>>,
    /// In-progress H3 connects, keyed like `inner` with `Transport::Quic`.
    #[cfg(feature = "http3")]
    pub(crate) inflight_h3: Mutex<HashMap<PoolKey, SharedH3Connect>>,
    /// `(host, port)` pairs that advertised `h3` in `Alt-Svc` or already completed a QUIC handshake; only these are raced.
    #[cfg(feature = "http3")]
    pub(crate) h3_known: Mutex<HashSet<(String, u16)>>,
    /// `(host, port, proxy)` triples whose TLS ALPN negotiated `http/1.1`; `Auto` skips the HTTP/2 attempt for them.
    pub(crate) h1_only: Mutex<HashSet<(String, u16, Option<String>)>>,
    pub(crate) idle_timeout: Duration,
    /// LRU cap.
    pub(crate) max_connections: usize,
    /// Max simultaneous HTTP/1.1 connections per `(host, port, proxy)`.
    pub(crate) max_h1_conns_per_host: usize,
    /// Per-host H1 connection permits.
    pub(crate) h1_permits: Mutex<HashMap<PoolKey, Arc<Semaphore>>>,
    pub(crate) counters: PoolCounters,
    /// Monotonic base for the idle-sweep throttle.
    created: Instant,
    /// Earliest `created.elapsed()` millis at which the next full idle/permit sweep may run.
    next_reap_ms: AtomicU64,
}

impl Pool {
    /// Remember that `host:port` via `proxy` negotiated `http/1.1`, so later `Auto` requests dial HTTP/1.1 directly.
    pub(crate) fn note_h1_only(&self, host: &str, port: u16, proxy: Option<&str>) {
        self.h1_only
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert((host.to_string(), port, proxy.map(str::to_string)));
    }

    /// `true` once `host:port` via `proxy` is known to speak HTTP/1.1 only.
    pub(crate) fn is_h1_only(&self, host: &str, port: u16, proxy: Option<&str>) -> bool {
        self.h1_only
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&(host.to_string(), port, proxy.map(str::to_string)))
    }

    /// Record that `host:port` speaks HTTP/3, from a completed QUIC handshake or an `Alt-Svc` header.
    #[cfg(feature = "http3")]
    pub(crate) fn note_h3(&self, host: &str, port: u16) {
        self.h3_known
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert((host.to_string(), port));
    }

    /// Record `host:port` as an HTTP/3 origin when an `Alt-Svc` header value advertises `h3`.
    #[cfg(feature = "http3")]
    pub(crate) fn note_alt_svc(&self, host: &str, port: u16, alt_svc: &str) {
        if alt_svc
            .split(',')
            .any(|alt| alt.trim_start().starts_with("h3="))
        {
            self.note_h3(host, port);
        }
    }

    /// `true` once `host:port` is known to speak HTTP/3.
    #[cfg(feature = "http3")]
    pub(crate) fn knows_h3(&self, host: &str, port: u16) -> bool {
        self.h3_known
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&(host.to_string(), port))
    }

    /// Create a pool with default 300 s idle timeout, a 2048-entry LRU cap, and the default per-host H1 connection cap.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            inflight_h2: Mutex::new(HashMap::new()),
            #[cfg(feature = "http3")]
            inflight_h3: Mutex::new(HashMap::new()),
            #[cfg(feature = "http3")]
            h3_known: Mutex::new(HashSet::new()),
            h1_only: Mutex::new(HashSet::new()),
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            max_h1_conns_per_host: DEFAULT_MAX_H1_CONNS_PER_HOST,
            h1_permits: Mutex::new(HashMap::new()),
            counters: PoolCounters::default(),
            created: Instant::now(),
            next_reap_ms: AtomicU64::new(0),
        }
    }

    /// Create a pool with explicit idle timeout, LRU cap, and per-host H1 connection cap.
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
            inflight_h2: Mutex::new(HashMap::new()),
            #[cfg(feature = "http3")]
            inflight_h3: Mutex::new(HashMap::new()),
            #[cfg(feature = "http3")]
            h3_known: Mutex::new(HashSet::new()),
            h1_only: Mutex::new(HashSet::new()),
            idle_timeout,
            max_connections,
            max_h1_conns_per_host,
            h1_permits: Mutex::new(HashMap::new()),
            counters: PoolCounters::default(),
            created: Instant::now(),
            next_reap_ms: AtomicU64::new(0),
        }
    }

    /// Return the in-progress H2 connect for `key`, or insert one built by `make` if none is running.
    pub(crate) fn inflight_h2_get_or_insert_with(
        &self,
        key: PoolKey,
        make: impl FnOnce() -> SharedConnect,
    ) -> SharedConnect {
        let mut map = self.inflight_h2.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = map.get(&key) {
            return existing.clone();
        }
        let shared = make();
        map.insert(key, shared.clone());
        shared
    }

    /// Remove the in-progress connect entry for `key` (idempotent).
    pub(crate) fn inflight_h2_remove(&self, key: &PoolKey) {
        self.inflight_h2
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
    }

    /// H3 analogue of [`Self::inflight_h2_get_or_insert_with`].
    #[cfg(feature = "http3")]
    pub(crate) fn inflight_h3_get_or_insert_with(
        &self,
        key: PoolKey,
        make: impl FnOnce() -> SharedH3Connect,
    ) -> SharedH3Connect {
        let mut map = self.inflight_h3.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = map.get(&key) {
            return existing.clone();
        }
        let shared = make();
        map.insert(key, shared.clone());
        shared
    }

    /// Remove the in-progress H3 connect entry for `key` (idempotent).
    #[cfg(feature = "http3")]
    pub(crate) fn inflight_h3_remove(&self, key: &PoolKey) {
        self.inflight_h3
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
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
            h3_hits: self.counters.h3_hits.load(Ordering::Relaxed),
            h3_misses: self.counters.h3_misses.load(Ordering::Relaxed),
            evictions_idle: self.counters.evictions_idle.load(Ordering::Relaxed),
            evictions_lru: self.counters.evictions_lru.load(Ordering::Relaxed),
            evictions_dead: self.counters.evictions_dead.load(Ordering::Relaxed),
            stale_probed: self.counters.stale_probed.load(Ordering::Relaxed),
            installs: self.counters.installs.load(Ordering::Relaxed),
        }
    }

    /// Evict idle connections AND any entry whose underlying handle is already dead.
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

    /// Drop the LRU entry when at or over cap.
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

    /// Look up a live H3 handle for `key`, touching its last-use timestamp.
    #[cfg(feature = "http3")]
    pub(crate) fn checkout_h3(&self, key: &PoolKey) -> Option<(H3Client, TlsInfo)> {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let dead = map.get(key).is_some_and(PooledConn::is_dead);
        if dead {
            map.remove(key);
            self.counters.evictions_dead.fetch_add(1, Ordering::Relaxed);
            self.counters.h3_misses.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        match map.get_mut(key) {
            Some(entry @ PooledConn::H3 { .. }) => {
                entry.set_last_use(Instant::now());
                if let PooledConn::H3 { handle, tls, .. } = entry {
                    let out = (handle.clone(), tls.clone());
                    self.counters.h3_hits.fetch_add(1, Ordering::Relaxed);
                    Some(out)
                } else {
                    unreachable!()
                }
            }
            _ => {
                self.counters.h3_misses.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    /// Install a freshly-opened H3 connection, or — if a live one already exists for `key` (a concurrent cold request beat us) — keep the existing one and return its handle, dropping ours.
    #[cfg(feature = "http3")]
    pub(crate) fn install_or_get_h3(
        &self,
        key: PoolKey,
        handle: H3Client,
        driver: H3DriverTask,
        tls: TlsInfo,
    ) -> (H3Client, TlsInfo) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(PooledConn::H3 {
            handle: existing,
            tls: existing_tls,
            ..
        }) = map.get(&key)
            && !existing.is_closed()
        {
            self.counters.h3_hits.fetch_add(1, Ordering::Relaxed);
            return (existing.clone(), existing_tls.clone());
        }
        if !map.contains_key(&key) {
            let evicted = Self::evict_lru_if_needed(&mut map, self.max_connections);
            if evicted > 0 {
                self.counters
                    .evictions_lru
                    .fetch_add(evicted, Ordering::Relaxed);
            }
        }
        let out = (handle.clone(), tls.clone());
        map.insert(
            key,
            PooledConn::H3 {
                handle,
                _driver: Some(driver),
                last_use: Instant::now(),
                tls,
            },
        );
        self.counters.installs.fetch_add(1, Ordering::Relaxed);
        out
    }

    /// Pop a warm idle H1 connection for `key`, if one is pooled.
    pub(crate) fn checkout_h1(&self, key: &PoolKey) -> Option<(H1Slot, TlsInfo)> {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match map.get_mut(key) {
            Some(PooledConn::H1 {
                idle,
                last_use,
                tls,
            }) => match idle.pop_back() {
                Some((slot, _returned_at)) => {
                    *last_use = Instant::now();
                    let tls = tls.clone();
                    self.counters.h1_hits.fetch_add(1, Ordering::Relaxed);
                    Some((slot, tls))
                }
                None => {
                    self.counters.h1_misses.fetch_add(1, Ordering::Relaxed);
                    None
                }
            },
            _ => {
                self.counters.h1_misses.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    /// Install a freshly-opened H2 connection, or — if a live one already exists for `key` (a concurrent cold request beat us) — keep the existing one and return its handle, dropping ours (its `DriverTask` drops here and GOAWAY-closes the unused connection).
    pub(crate) fn install_h2(
        &self,
        key: PoolKey,
        handle: H2Client,
        driver: DriverTask,
        tls: TlsInfo,
    ) -> (H2Client, TlsInfo) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(PooledConn::H2 {
            handle: existing,
            tls: existing_tls,
            ..
        }) = map.get(&key)
            && !existing.is_closed()
        {
            self.counters.h2_hits.fetch_add(1, Ordering::Relaxed);
            return (existing.clone(), existing_tls.clone());
        }
        if !map.contains_key(&key) {
            let evicted = Self::evict_lru_if_needed(&mut map, self.max_connections);
            if evicted > 0 {
                self.counters
                    .evictions_lru
                    .fetch_add(evicted, Ordering::Relaxed);
            }
        }
        let out = (handle.clone(), tls.clone());
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
        out
    }

    /// Return a still-reusable H1 connection to the pool for `key`.
    pub(crate) fn return_h1(&self, key: PoolKey, slot: H1Slot, tls: TlsInfo) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match map.get_mut(&key) {
            Some(PooledConn::H1 { idle, last_use, .. }) => {
                idle.push_back((slot, Instant::now()));
                *last_use = Instant::now();
                return;
            }
            Some(PooledConn::H2 { .. }) => return,
            #[cfg(feature = "http3")]
            Some(PooledConn::H3 { .. }) => return,
            None => {}
        }
        let evicted = Self::evict_lru_if_needed(&mut map, self.max_connections);
        if evicted > 0 {
            self.counters
                .evictions_lru
                .fetch_add(evicted, Ordering::Relaxed);
        }
        let mut idle = VecDeque::new();
        idle.push_back((slot, Instant::now()));
        map.insert(
            key,
            PooledConn::H1 {
                idle,
                last_use: Instant::now(),
                tls,
            },
        );
    }

    /// Acquire a per-host H1 connection permit, waiting if all `max_h1_conns_per_host` are in use.
    pub(crate) async fn acquire_h1_permit(&self, key: &PoolKey) -> OwnedSemaphorePermit {
        let sem = {
            let mut permits = self.h1_permits.lock().unwrap_or_else(|e| e.into_inner());
            permits
                .entry(key.clone())
                .or_insert_with(|| Arc::new(Semaphore::new(self.max_h1_conns_per_host)))
                .clone()
        };
        sem.acquire_owned()
            .await
            .expect("h1 per-host semaphore is never closed")
    }

    /// Record that a freshly-opened H1 connection was installed into the pool (stats only).
    pub(crate) fn note_h1_install(&self) {
        self.counters.installs.fetch_add(1, Ordering::Relaxed);
    }

    /// Record that a checked-out H1 connection died mid-request and was discarded (stats only).
    pub(crate) fn note_h1_dead(&self) {
        self.counters.evictions_dead.fetch_add(1, Ordering::Relaxed);
    }

    /// Record that the checkout liveness probe found a popped H1 connection already dead and discarded it before committing a request (stats only).
    pub(crate) fn note_h1_stale_probed(&self) {
        self.counters.stale_probed.fetch_add(1, Ordering::Relaxed);
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

    /// Fill with `n` synthetic live H2 entries sharing `handle` (driver: None; liveness via the shared driver).
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
                    _driver: None,
                    last_use: Instant::now(),
                    tls: TlsInfo::default(),
                },
            );
        }
    }

    /// `checkout_handle`'s hot-path body: make_key + evict_idle + checkout_h2.
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

#[cfg(test)]
mod tests;
