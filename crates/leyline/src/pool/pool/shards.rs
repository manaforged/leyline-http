use std::hash::{BuildHasher, Hash};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, RwLock};

use foldhash::HashMap;
use foldhash::fast::RandomState;

use super::slots::H1Slots;
use crate::pool::types::{Entries, PoolKey};

const MIN_ENTRIES_PER_SHARD: usize = 16;
const MAX_SHARDS: usize = 64;

#[derive(Default)]
pub(super) struct Shard {
    pub(super) entries: RwLock<Entries>,
    pub(super) h1_permits: Mutex<HashMap<PoolKey, H1Slots>>,
}

pub(super) struct Shards {
    list: Box<[Shard]>,
    hasher: RandomState,
    total: AtomicUsize,
    limit: usize,
}

impl Shards {
    pub(super) fn new(max_connections: usize) -> Self {
        let wanted = (max_connections / MIN_ENTRIES_PER_SHARD).clamp(1, MAX_SHARDS);
        let count = 1 << wanted.ilog2();
        Self {
            list: (0..count).map(|_| Shard::default()).collect(),
            hasher: RandomState::default(),
            total: AtomicUsize::new(0),
            limit: max_connections,
        }
    }

    pub(super) fn of(&self, key: &impl Hash) -> &Shard {
        let at = self.hasher.hash_one(key) as usize & (self.list.len() - 1);
        &self.list[at]
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &Shard> {
        self.list.iter()
    }

    pub(super) fn added(&self) {
        self.total.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn removed(&self, count: usize) {
        self.total.fetch_sub(count, Ordering::Relaxed);
    }

    pub(super) fn over_limit_with_one_more(&self) -> usize {
        (self.total.load(Ordering::Relaxed) + 1).saturating_sub(self.limit)
    }
}
