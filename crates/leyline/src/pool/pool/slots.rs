use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::Pool;
use crate::h2::client::H2Client;
use crate::pool::types::{H1Slot, PoolKey, PooledConn, TlsInfo};
#[cfg(feature = "http3")]
use crate::quic::H3Client;
use crate::util::lock;

impl Pool {
    pub(crate) fn checkout_h2(&self, key: &PoolKey) -> Option<(H2Client, TlsInfo)> {
        let mut map = lock(&self.inner);
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

    #[cfg(feature = "http3")]
    pub(crate) fn checkout_h3(&self, key: &PoolKey) -> Option<(H3Client, TlsInfo)> {
        let mut map = lock(&self.inner);
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

    #[cfg(feature = "http3")]
    pub(crate) fn install_or_get_h3(
        &self,
        key: PoolKey,
        handle: H3Client,
        tls: TlsInfo,
    ) -> (H3Client, TlsInfo) {
        self.clear_h3_broken(&key.host, key.port, key.proxy.as_deref());
        let mut map = lock(&self.inner);
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
                last_use: Instant::now(),
                tls,
            },
        );
        self.counters.installs.fetch_add(1, Ordering::Relaxed);
        out
    }

    pub(crate) fn checkout_h1(&self, key: &PoolKey) -> Option<(H1Slot, TlsInfo)> {
        let mut map = lock(&self.inner);
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

    pub(crate) fn install_h2(
        &self,
        key: PoolKey,
        handle: H2Client,
        tls: TlsInfo,
    ) -> (H2Client, TlsInfo) {
        let mut map = lock(&self.inner);
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
                last_use: Instant::now(),
                tls,
            },
        );
        self.counters.installs.fetch_add(1, Ordering::Relaxed);
        out
    }

    pub(crate) fn return_h1(&self, key: PoolKey, slot: H1Slot, tls: TlsInfo) {
        let mut map = lock(&self.inner);
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

    pub(crate) async fn acquire_h1_permit(&self, key: &PoolKey) -> OwnedSemaphorePermit {
        let sem = {
            let mut permits = lock(&self.h1_permits);
            permits
                .entry(PoolKey {
                    partition: 0,
                    ..key.clone()
                })
                .or_insert_with(|| Arc::new(Semaphore::new(self.max_h1_conns_per_host)))
                .clone()
        };
        sem.acquire_owned()
            .await
            .expect("h1 per-host semaphore is never closed")
    }
}
