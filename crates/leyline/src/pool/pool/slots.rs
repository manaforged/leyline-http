use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::Pool;
use crate::pool::types::{
    Counter, H1Idle, H1Slot, MuxClient, PoolKey, PooledConn, Proto, Stamp, TlsInfo,
};
use crate::util::{read, write};

impl Pool {
    pub(crate) fn checkout_mux<C: MuxClient>(&self, key: &PoolKey) -> Option<(C, TlsInfo)> {
        self.checkout_mux_idle(key)
            .map(|(conn, tls, _)| (conn, tls))
    }

    pub(crate) fn checkout_mux_idle<C: MuxClient>(
        &self,
        key: &PoolKey,
    ) -> Option<(C, TlsInfo, Duration)> {
        let mut dead = false;
        let hit = match read(&self.inner).get(key) {
            Some(entry) if entry.is_dead() => {
                dead = true;
                None
            }
            Some(PooledConn::Mux {
                handle,
                last_use,
                tls,
            }) => C::in_handle(handle)
                .map(|conn| (conn.clone(), tls.clone(), last_use.touch(Instant::now()))),
            _ => None,
        };
        if dead {
            let mut map = write(&self.inner);
            if map.get(key).is_some_and(PooledConn::is_dead) {
                map.remove(key);
                self.counters.add(Counter::EvictionsDead, 1);
            }
        }
        hit
    }

    pub(crate) fn install_mux<C: MuxClient>(
        &self,
        key: PoolKey,
        handle: C,
        tls: TlsInfo,
    ) -> (C, TlsInfo) {
        let mut map = write(&self.inner);
        if let Some(PooledConn::Mux {
            handle: existing,
            tls: existing_tls,
            ..
        }) = map.get(&key)
            && !existing.is_closed()
            && let Some(existing) = C::in_handle(existing)
        {
            return (existing.clone(), existing_tls.clone());
        }
        if !map.contains_key(&key) {
            let evicted = Self::evict_lru_if_needed(&mut map, self.config.max_connections);
            if evicted > 0 {
                self.counters.add(Counter::EvictionsLru, evicted);
            }
        }
        let out = (handle.clone(), tls.clone());
        map.insert(
            key,
            PooledConn::Mux {
                handle: handle.into_handle(),
                last_use: Stamp::now(),
                tls,
            },
        );
        self.counters.add(Counter::Installs, 1);
        out
    }

    pub(crate) fn checkout_h1(&self, key: &PoolKey) -> Option<(H1Slot, TlsInfo)> {
        let mut map = write(&self.inner);
        let hit = match map.get_mut(key) {
            Some(PooledConn::H1 {
                idle,
                last_use,
                tls,
            }) => idle.queue().pop_back().map(|(slot, _returned_at)| {
                *last_use = Instant::now();
                (slot, tls.clone())
            }),
            _ => None,
        };
        self.counters.checkout(Proto::H1, hit.is_some());
        hit
    }

    pub(crate) fn return_h1(&self, key: PoolKey, slot: H1Slot, tls: TlsInfo) {
        let mut map = write(&self.inner);
        match map.get_mut(&key) {
            Some(PooledConn::H1 { idle, last_use, .. }) => {
                idle.queue().push_back((slot, Instant::now()));
                *last_use = Instant::now();
                return;
            }
            Some(PooledConn::Mux { .. }) => return,
            None => {}
        }
        let evicted = Self::evict_lru_if_needed(&mut map, self.config.max_connections);
        if evicted > 0 {
            self.counters.add(Counter::EvictionsLru, evicted);
        }
        let mut idle = H1Idle::default();
        idle.queue().push_back((slot, Instant::now()));
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
        let max_per_host = self.config.max_h1_conns_per_host;
        let sem = {
            let mut permits = crate::util::lock(&self.h1_permits);
            permits
                .entry(PoolKey {
                    partition: 0,
                    ..key.clone()
                })
                .or_insert_with(|| Arc::new(Semaphore::new(max_per_host)))
                .clone()
        };
        sem.acquire_owned()
            .await
            .expect("h1 per-host semaphore is never closed")
    }
}
