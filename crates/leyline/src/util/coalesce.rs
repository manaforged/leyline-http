use std::future::Future;
use std::hash::Hash;
use std::sync::{Arc, Mutex};

use foldhash::HashMap;
use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};

use super::lock;

/// A shared result that later callers for the same key join while it runs.
pub(crate) type Joined<V> = Shared<BoxFuture<'static, V>>;

/// Runs at most one task per key and shares its result with every caller that asks while it runs.
pub(crate) struct Coalesce<K, V>(Arc<Mutex<HashMap<K, Joined<V>>>>);

impl<K, V> Clone for Coalesce<K, V> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<K, V> Default for Coalesce<K, V> {
    fn default() -> Self {
        Self(Arc::default())
    }
}

impl<K, V> Coalesce<K, V>
where
    K: Hash + Eq + Clone + Send + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// Joins the task running for `key`, or spawns `open` and forgets it once it finishes.
    pub(crate) fn join_or_spawn<F>(
        &self,
        key: &K,
        open: impl FnOnce() -> F,
        panicked: impl FnOnce(tokio::task::JoinError) -> V + Send + 'static,
    ) -> Joined<V>
    where
        F: Future<Output = V> + Send + 'static,
    {
        let mut guard = RemoveOnDrop {
            registry: Arc::clone(&self.0),
            key: Some(key.clone()),
        };
        let mut map = lock(&self.0);
        if let Some(existing) = map.get(key) {
            guard.key = None;
            return existing.clone();
        }
        let open = open();
        let handle = tokio::spawn(async move {
            let _guard = guard;
            open.await
        });
        let shared = async move { handle.await.unwrap_or_else(panicked) }
            .boxed()
            .shared();
        map.insert(key.clone(), shared.clone());
        shared
    }
}

struct RemoveOnDrop<K: Hash + Eq, V> {
    registry: Arc<Mutex<HashMap<K, Joined<V>>>>,
    key: Option<K>,
}

impl<K: Hash + Eq, V> Drop for RemoveOnDrop<K, V> {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            lock(&self.registry).remove(&key);
        }
    }
}
