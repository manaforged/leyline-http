use std::collections::HashMap;
use std::hash::Hash;
use std::time::SystemTime;

pub(crate) const MAX_ENTRIES: usize = 1024;

pub(crate) struct ExpiringSet<K> {
    entries: HashMap<K, SystemTime>,
}

impl<K> Default for ExpiringSet<K> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

impl<K: Eq + Hash + Clone> ExpiringSet<K> {
    pub(crate) fn insert(&mut self, key: K, expiry: SystemTime, now: SystemTime) {
        if !self.entries.contains_key(&key) && self.entries.len() >= MAX_ENTRIES {
            self.make_room(now);
        }
        self.entries.insert(key, expiry);
    }

    #[cfg(feature = "http3")]
    pub(crate) fn remove(&mut self, key: &K) {
        self.entries.remove(key);
    }

    pub(crate) fn contains(&mut self, key: &K, now: SystemTime) -> bool {
        match self.entries.get(key) {
            Some(expiry) if *expiry > now => true,
            Some(_) => {
                self.entries.remove(key);
                false
            }
            None => false,
        }
    }

    fn make_room(&mut self, now: SystemTime) {
        self.entries.retain(|_, expiry| *expiry > now);
        if self.entries.len() < MAX_ENTRIES {
            return;
        }
        if let Some(oldest) = self
            .entries
            .iter()
            .min_by_key(|(_, expiry)| **expiry)
            .map(|(key, _)| key.clone())
        {
            self.entries.remove(&oldest);
        }
    }
}
