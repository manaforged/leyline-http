use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::Pool;
use crate::error::Error;
use crate::pool::H1Slot;
use crate::pool::types::{
    Counter, Entries, Evicted, H1Entry, H1Idle, MuxClient, MuxEntry, PoolKey, Proto, Stamp,
};
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
        let shard = self.conns.shards.of(key);
        let hit = match read(&shard.entries).mux.get(key) {
            Some(entry) if entry.is_dead() || self.past_lifetime(entry.opened, now) => {
                dead = true;
                None
            }
            Some(entry) => C::in_handle(&entry.handle)
                .map(|conn| (conn.clone(), entry.tls.clone(), entry.last_use.touch(now))),
            None => None,
        };
        if dead {
            let mut entries = write(&shard.entries);
            let stale = |entry: &MuxEntry| entry.is_dead() || self.past_lifetime(entry.opened, now);
            if entries.mux.get(key).is_some_and(stale) {
                entries.mux.remove(key);
                self.conns.shards.removed(1);
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
        let mut entries = write(&self.conns.shards.of(&key).entries);
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
        let evicted = if replaces {
            Vec::new()
        } else {
            self.evict_for_new_entry(&mut entries)
        };
        let out = (handle.clone(), tls.clone());
        entries.mux.insert(
            key,
            MuxEntry {
                handle: handle.into_handle(),
                opened: Instant::now(),
                last_use: Stamp::now(),
                tls,
            },
        );
        drop(entries);
        drop(evicted);
        self.conns.counters.add(Counter::Installs, 1);
        self.watch_idle();
        out
    }

    pub(crate) fn checkout_h1(&self, key: &PoolKey, now: Instant) -> Option<(H1Slot, TlsInfo)> {
        let mut entries = write(&self.conns.shards.of(key).entries);
        let hit = entries.h1.get_mut(key).and_then(|entry| {
            while let Some((slot, _returned_at)) = entry.idle.queue().pop_back() {
                if !self.past_lifetime(slot.opened_at, now) {
                    entry.last_use = now;
                    return Some((slot, entry.tls.clone()));
                }
            }
            None
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
        if self.past_lifetime(slot.opened_at, Instant::now()) {
            return;
        }
        self.park_h1_locked(key, slot, tls);
        self.watch_idle();
    }

    fn park_h1_locked(&self, key: &PoolKey, slot: H1Slot, tls: TlsInfo) {
        let mut entries = write(&self.conns.shards.of(key).entries);
        if let Some(entry) = entries.h1.get_mut(key) {
            let now = Instant::now();
            entry.idle.queue().push_back((slot, now));
            entry.last_use = now;
            return;
        }
        if entries.mux.contains_key(key) {
            return;
        }
        let evicted = self.evict_for_new_entry(&mut entries);
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
        drop(entries);
        drop(evicted);
    }

    fn past_lifetime(&self, opened: Instant, now: Instant) -> bool {
        self.config
            .max_lifetime
            .is_some_and(|lifetime| now.duration_since(opened) >= lifetime)
    }

    fn evict_for_new_entry(&self, entries: &mut Entries) -> Vec<Evicted> {
        let shards = &self.conns.shards;
        let evicted = entries.evict_lru(shards.over_limit_with_one_more());
        if !evicted.is_empty() {
            shards.removed(evicted.len());
            self.conns
                .counters
                .add(Counter::EvictionsLru, evicted.len() as u64);
        }
        shards.added();
        evicted
    }

    pub(crate) async fn acquire_h1_permit(
        &self,
        key: &PoolKey,
    ) -> Result<OwnedSemaphorePermit, Error> {
        let max_per_host = self.config.max_h1_conns_per_host;
        let host = key.host_key();
        let slots = {
            let mut permits = crate::util::lock(&self.conns.shards.of(&*host).h1_permits);
            match permits.get(&*host).cloned() {
                Some(slots) => slots,
                None => permits
                    .entry(host.clone().into_owned())
                    .or_insert_with(|| H1Slots::new(max_per_host))
                    .clone(),
            }
        };
        if let Ok(permit) = Arc::clone(&slots.permits).try_acquire_owned() {
            return Ok(permit);
        }
        let _pending = Pending::enter(&slots.pending, self.config.max_h1_pending_per_host)?;
        Ok(slots
            .permits
            .acquire_owned()
            .await
            .expect("h1 per-host semaphore is never closed"))
    }
}

#[derive(Clone)]
pub(super) struct H1Slots {
    pub(super) permits: Arc<Semaphore>,
    pending: Arc<AtomicUsize>,
}

impl H1Slots {
    fn new(max_per_host: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(max_per_host)),
            pending: Arc::default(),
        }
    }
}

struct Pending(Arc<AtomicUsize>);

impl Pending {
    fn enter(count: &Arc<AtomicUsize>, limit: Option<usize>) -> Result<Self, Error> {
        let held = Self(Arc::clone(count));
        let before = count.fetch_add(1, Ordering::Relaxed);
        if limit.is_some_and(|limit| before >= limit) {
            return Err(Error::overloaded());
        }
        Ok(held)
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}
