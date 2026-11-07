use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::Pool;
use crate::pool::H1Slot;
use crate::pool::types::{Counter, H1Entry, H1Idle, MuxClient, MuxEntry, PoolKey, Proto, Stamp};
use crate::util::{read, write};
use crate::wire::TlsInfo;

impl Pool {
    pub(crate) fn checkout_mux<C: MuxClient>(&self, key: &PoolKey) -> Option<(C, TlsInfo)> {
        self.checkout_mux_idle(key, Instant::now())
            .map(|(conn, tls, _)| (conn, tls))
    }

    pub(crate) fn checkout_mux_idle<C: MuxClient>(
        &self,
        key: &PoolKey,
        now: Instant,
    ) -> Option<(C, TlsInfo, Duration)> {
        let mut dead = false;
        let hit = match read(&self.conns.entries).mux.get(key) {
            Some(entry) if entry.is_dead() => {
                dead = true;
                None
            }
            Some(entry) => C::in_handle(&entry.handle)
                .map(|conn| (conn.clone(), entry.tls.clone(), entry.last_use.touch(now))),
            None => None,
        };
        if dead {
            let mut entries = write(&self.conns.entries);
            if entries.mux.get(key).is_some_and(MuxEntry::is_dead) {
                entries.mux.remove(key);
                self.conns.counters.add(Counter::EvictionsDead, 1);
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
        let mut entries = write(&self.conns.entries);
        if let Some(existing) = entries.mux.get(&key)
            && !existing.is_dead()
            && let Some(conn) = C::in_handle(&existing.handle)
        {
            return (conn.clone(), existing.tls.clone());
        }
        if !self.config.keepalive {
            return (handle, tls);
        }
        let replaces = entries.h1.remove(&key).is_some() || entries.mux.contains_key(&key);
        if !replaces {
            let evicted = entries.evict_lru(self.config.max_connections);
            if evicted > 0 {
                self.conns.counters.add(Counter::EvictionsLru, evicted);
            }
        }
        let out = (handle.clone(), tls.clone());
        entries.mux.insert(
            key,
            MuxEntry {
                handle: handle.into_handle(),
                last_use: Stamp::now(),
                tls,
            },
        );
        self.conns.counters.add(Counter::Installs, 1);
        out
    }

    pub(crate) fn checkout_h1(&self, key: &PoolKey, now: Instant) -> Option<(H1Slot, TlsInfo)> {
        let mut entries = write(&self.conns.entries);
        let hit = entries.h1.get_mut(key).and_then(|entry| {
            entry.idle.queue().pop_back().map(|(slot, _returned_at)| {
                entry.last_use = now;
                (slot, entry.tls.clone())
            })
        });
        self.conns.counters.checkout(Proto::H1, hit.is_some());
        hit
    }

    pub(crate) fn return_h1(&self, key: &PoolKey, slot: H1Slot, tls: TlsInfo) -> bool {
        if !self.config.keepalive {
            return false;
        }
        self.park_h1(key, slot, tls);
        true
    }

    pub(crate) fn park_h1(&self, key: &PoolKey, slot: H1Slot, tls: TlsInfo) {
        let mut entries = write(&self.conns.entries);
        if let Some(entry) = entries.h1.get_mut(key) {
            let now = Instant::now();
            entry.idle.queue().push_back((slot, now));
            entry.last_use = now;
            return;
        }
        if entries.mux.contains_key(key) {
            return;
        }
        let evicted = entries.evict_lru(self.config.max_connections);
        if evicted > 0 {
            self.conns.counters.add(Counter::EvictionsLru, evicted);
        }
        let mut idle = H1Idle::default();
        idle.queue().push_back((slot, Instant::now()));
        entries.h1.insert(
            key.clone(),
            H1Entry {
                idle,
                last_use: Instant::now(),
                tls,
            },
        );
    }

    pub(crate) async fn acquire_h1_permit(&self, key: &PoolKey) -> OwnedSemaphorePermit {
        let max_per_host = self.config.max_h1_conns_per_host;
        let sem = {
            let mut permits = crate::util::lock(&self.conns.h1_permits);
            let host = key.host_key();
            match permits.get(&*host).cloned() {
                Some(sem) => sem,
                None => permits
                    .entry(host.into_owned())
                    .or_insert_with(|| Arc::new(Semaphore::new(max_per_host)))
                    .clone(),
            }
        };
        sem.acquire_owned()
            .await
            .expect("h1 per-host semaphore is never closed")
    }
}
