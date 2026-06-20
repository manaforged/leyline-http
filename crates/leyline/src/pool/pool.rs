//! [`Pool`] struct — thread-safe connection map with LRU eviction.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::future::{BoxFuture, Shared};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::h2::client::{DriverTask, H2Client};

use crate::pool::types::{H1Slot, PoolCounters, PoolKey, PoolStats, PooledConn, TlsInfo};

/// A shared, awaitable in-progress H2 connect. Concurrent first-requests to the same
/// destination clone this handle and await ONE connect (single-flight) instead of each
/// opening its own — preventing the connection storm that a one-conn-per-host pool otherwise
/// triggers at high concurrency. The `Output` is `Clone` (both `H2Client` and `TlsInfo` are
/// cheap clones; the error is `Arc`-wrapped) as `Shared` requires.
pub(crate) type SharedConnect =
    Shared<BoxFuture<'static, Result<(H2Client, TlsInfo), Arc<crate::Error>>>>;

/// Default idle-timeout for pooled connections.
///
/// Matches real Chrome's `kUsedIdleSocketTimeout` (5 minutes) — the
/// timeout Chromium applies to a pooled socket that has already
/// served at least one request. The original 90s default was
/// significantly tighter than browser behavior and forced every
/// long-lived caller (long-lived / session-persistent pooled
/// workloads) into 60s app-level keep-alive pings
/// just to outrun the pool reaper. At 5 minutes leyline behaves
/// like a real Chrome network stack: idle but not yet abandoned
/// connections sit in the pool, ready for the next request, until
/// either a server-side GOAWAY closes them or the LRU cap evicts
/// them.
///
/// Override via `Session::builder().pool_idle_timeout(...)` when
/// the caller has a different reuse profile (e.g. one-shot
/// clients).
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// Default LRU cap: 2048 entries.
pub const DEFAULT_MAX_CONNECTIONS: usize = 2048;

/// Default cap on simultaneous HTTP/1.1 connections **per destination**
/// `(host, port, proxy)`.
///
/// HTTP/1.1 cannot multiplex, so concurrent requests to one host need
/// separate connections; this ceiling bounds how many open at once
/// (requests beyond it wait for one to free rather than opening another).
///
/// The default favours **throughput**, set to Chrome's *total* socket
/// ceiling (`kMaxSockets`, 256) rather than its per-host-group limit
/// (`kMaxSocketsPerGroup`, 6). Reasoning: HTTP/1.1 is leyline's ALPN
/// fallback — a real Chrome speaks HTTP/2 to virtually every modern host —
/// so a server that sees us on H1 *at all* already sees a non-Chrome-typical
/// client; the per-host H1 socket *count* is a fingerprint signal that is
/// moot by the time it would apply. At 256 leyline opens as many H1
/// connections as concurrency demands (matching the uncapped Rust/Go peers
/// for a like-for-like comparison) while still bounding pathological socket
/// storms.
///
/// For strict per-host-socket fidelity (exactly mirror a browser's 6),
/// set `Session::builder().h1_max_conns_per_host(6)`. The HTTP/2 path is
/// unaffected (one multiplexed connection per host).
pub const DEFAULT_MAX_H1_CONNS_PER_HOST: usize = 256;

/// HTTP connection pool.
///
/// Thread-safe (`Arc<Mutex<>>`) — a `Session` holds `Arc<Pool>` so
/// cloned sessions share the same pool. Entries are keyed by
/// `(host, port, proxy)` and may be either HTTP/2 (multiplexed
/// clone-handle) or HTTP/1.1 keep-alive (single-checkout owned
/// stream).
pub struct Pool {
    pub(crate) inner: Mutex<HashMap<PoolKey, PooledConn>>,
    /// In-progress H2 connects, keyed like `inner`. Single-flights connection establishment so
    /// concurrent first-requests to one destination share a single TLS+H2 handshake.
    pub(crate) inflight_h2: Mutex<HashMap<PoolKey, SharedConnect>>,
    pub(crate) idle_timeout: Duration,
    /// LRU cap.
    pub(crate) max_connections: usize,
    /// Max simultaneous HTTP/1.1 connections per `(host, port, proxy)`.
    pub(crate) max_h1_conns_per_host: usize,
    /// Per-host H1 connection permits. One `Semaphore(max_h1_conns_per_host)`
    /// per destination caps how many H1 connections run at once and queues
    /// the overflow — the async wait primitive the synchronous `inner` map
    /// cannot provide. A permit is held for the whole request/response
    /// exchange and released (on drop) when the connection is returned to
    /// `inner` or closed, so the live-connection count never exceeds the cap.
    pub(crate) h1_permits: Mutex<HashMap<PoolKey, Arc<Semaphore>>>,
    pub(crate) counters: PoolCounters,
}

impl Pool {
    /// Create a pool with default 300 s idle timeout, a 2048-entry LRU cap,
    /// and the default per-host H1 connection cap.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            inflight_h2: Mutex::new(HashMap::new()),
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            max_h1_conns_per_host: DEFAULT_MAX_H1_CONNS_PER_HOST,
            h1_permits: Mutex::new(HashMap::new()),
            counters: PoolCounters::default(),
        }
    }

    /// Create a pool with explicit idle timeout, LRU cap, and per-host H1
    /// connection cap.
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
            idle_timeout,
            max_connections,
            max_h1_conns_per_host,
            h1_permits: Mutex::new(HashMap::new()),
            counters: PoolCounters::default(),
        }
    }

    /// Return the in-progress H2 connect for `key`, or insert one built by `make` if none is
    /// running. `make` runs only on a miss, under the lock, and must not await. The returned
    /// handle is awaited by the caller; every concurrent caller for the same key awaits the
    /// same connect. The connect future is responsible for removing its own entry on completion
    /// (see `inflight_remove`), so a cancelled leader cannot leave a stale entry.
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
    /// already dead. Called before every checkout. Also prunes idle per-host
    /// H1 semaphores so `h1_permits` stays bounded like the rest of the pool.
    ///
    /// For H1 entries this trims **per connection**: each warm connection idle
    /// past the timeout is dropped from the deque and counted individually. An
    /// entry is removed only when idle expiry actually emptied it — an entry
    /// whose deque is empty merely because all its connections are checked out
    /// (in-flight) dropped nothing and is kept, so a host running at its cap
    /// does not churn (and over-count) a map entry on every request. H2 entries
    /// evict whole, as before.
    pub(crate) fn evict_idle(&self) {
        let now = Instant::now();
        let mut idle_evicted = 0u64;
        {
            let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            map.retain(|_, entry| match entry {
                PooledConn::H1 { idle, .. } => {
                    let before = idle.len();
                    idle.retain(|(_, returned_at)| {
                        now.duration_since(*returned_at) < self.idle_timeout
                    });
                    idle_evicted += (before - idle.len()) as u64;
                    // Keep unless idle expiry actually emptied the deque; an
                    // entry emptied only by in-flight checkouts dropped nothing
                    // (before == len) and must survive for the returning
                    // requests to reuse.
                    !idle.is_empty() || before == idle.len()
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
            });
        }
        // Prune per-host H1 semaphores with no in-flight users or queued
        // waiters. `Arc::strong_count == 1` means only this map holds the
        // `Arc<Semaphore>` — every `OwnedSemaphorePermit` and every request
        // mid-`acquire` holds its own clone — so the destination is idle and the
        // semaphore can be dropped and lazily recreated on the next request.
        // Without this `h1_permits` would grow one entry per distinct
        // destination forever; this keeps it bounded to recently-active hosts,
        // matching the LRU + idle bound on `inner`.
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

    /// Pop a warm idle H1 connection for `key`, if one is pooled.
    ///
    /// Returns the most-recently-returned connection (LIFO via `pop_back`:
    /// the freshest, most likely still alive). The caller holds an H1 permit
    /// (see [`Self::acquire_h1_permit`]) for the whole exchange, so the
    /// per-host connection count is already bounded; a miss here just means
    /// the caller opens a fresh connection under that same permit.
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

    /// Return a still-reusable H1 connection to the pool for `key`.
    ///
    /// Pushes the connection into the destination's warm-idle deque so the
    /// next request to that host reuses it instead of opening a fresh one.
    /// Creates the entry on first return (LRU-evicting another destination
    /// if inserting a new key would exceed the cap). If the key already
    /// holds an H2 entry — only possible if a host flipped protocols — the
    /// H1 connection is dropped rather than clobbering the H2 entry.
    ///
    /// The deque cannot exceed `max_h1_conns_per_host`: a connection only
    /// reaches here while its caller still holds one of that host's permits,
    /// and there are exactly `max_h1_conns_per_host` permits.
    pub(crate) fn return_h1(&self, key: PoolKey, slot: H1Slot, tls: TlsInfo) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match map.get_mut(&key) {
            Some(PooledConn::H1 { idle, last_use, .. }) => {
                idle.push_back((slot, Instant::now()));
                *last_use = Instant::now();
                return;
            }
            // Key holds an H2 entry (protocol flip) — don't clobber it; drop.
            Some(PooledConn::H2 { .. }) => return,
            None => {}
        }
        // First connection to this destination — create the entry.
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

    /// Acquire a per-host H1 connection permit, waiting if all
    /// `max_h1_conns_per_host` are in use. The returned guard is held for the
    /// whole request/response exchange; dropping it frees the slot for a
    /// queued request. This bounds the live H1 connection count per
    /// destination and gives the overflow somewhere to wait — the browser
    /// per-host socket-pool model.
    pub(crate) async fn acquire_h1_permit(&self, key: &PoolKey) -> OwnedSemaphorePermit {
        let sem = {
            let mut permits = self.h1_permits.lock().unwrap_or_else(|e| e.into_inner());
            permits
                .entry(key.clone())
                .or_insert_with(|| Arc::new(Semaphore::new(self.max_h1_conns_per_host)))
                .clone()
        };
        // The per-host semaphore is never closed, so the only documented
        // failure mode of `acquire_owned` cannot occur here.
        sem.acquire_owned()
            .await
            .expect("h1 per-host semaphore is never closed")
    }

    /// Record that a freshly-opened H1 connection was installed into the
    /// pool (stats only). Counts a connection only once it is actually
    /// pooled for reuse — a connection that completes non-reusably (e.g. a
    /// framing conflict) is opened but never installed, so it is not counted.
    pub(crate) fn note_h1_install(&self) {
        self.counters.installs.fetch_add(1, Ordering::Relaxed);
    }

    /// Record that a checked-out H1 connection died mid-request and was
    /// discarded (stats only). The connection was already removed from its
    /// destination's deque by [`Self::checkout_h1`], so unlike the old
    /// single-slot pool this does **not** evict the whole entry — sibling
    /// warm connections to the same host stay pooled. It only counts the
    /// one dead connection.
    pub(crate) fn note_h1_dead(&self) {
        self.counters.evictions_dead.fetch_add(1, Ordering::Relaxed);
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
