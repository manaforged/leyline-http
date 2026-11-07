use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::core::FetchSite;
use crate::profile::RequestKind;
use crate::wire::{HeaderBlock, HeaderPair, RequestHeaders};

/// A session state id that changes whenever the session state is copied.
pub(crate) struct HeaderId(u64);

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

impl HeaderId {
    pub(crate) fn get(&self) -> u64 {
        self.0
    }
}

impl Default for HeaderId {
    fn default() -> Self {
        Self(NEXT_ID.fetch_add(1, Ordering::Relaxed))
    }
}

impl Clone for HeaderId {
    fn clone(&self) -> Self {
        Self::default()
    }
}

/// The request facts that a cacheable header list depends on.
pub(crate) struct HeaderKey<'a> {
    pub(crate) session: u64,
    pub(crate) preset: Option<RequestKind>,
    pub(crate) method: &'a str,
    pub(crate) origin: &'a str,
    pub(crate) referer: &'a str,
    pub(crate) site: FetchSite,
    pub(crate) downgrade: bool,
}

struct CachedKey {
    session: u64,
    preset: Option<RequestKind>,
    method: Box<str>,
    origin: Box<str>,
    referer: Box<str>,
    site: FetchSite,
    downgrade: bool,
}

impl CachedKey {
    fn new(key: &HeaderKey<'_>) -> Self {
        Self {
            session: key.session,
            preset: key.preset,
            method: key.method.into(),
            origin: key.origin.into(),
            referer: key.referer.into(),
            site: key.site,
            downgrade: key.downgrade,
        }
    }

    fn matches(&self, key: &HeaderKey<'_>) -> bool {
        self.session == key.session
            && self.preset == key.preset
            && self.site == key.site
            && self.downgrade == key.downgrade
            && *self.method == *key.method
            && *self.origin == *key.origin
            && *self.referer == *key.referer
    }
}

const CACHE_ENTRIES: usize = 8;

thread_local! {
    static CACHE: RefCell<Vec<(CachedKey, Arc<HeaderBlock>)>> = const { RefCell::new(Vec::new()) };
}

/// Returns the header list for `key` from this thread's cache, building and storing it on a miss.
pub(crate) fn cached(
    key: &HeaderKey<'_>,
    build: impl FnOnce() -> Vec<HeaderPair>,
) -> RequestHeaders {
    if let Some(block) = lookup(key) {
        return RequestHeaders::Shared(block);
    }
    let block = Arc::new(HeaderBlock::new(build()));
    CACHE.with_borrow_mut(|cache| {
        if cache.len() == CACHE_ENTRIES {
            cache.pop();
        }
        cache.insert(0, (CachedKey::new(key), Arc::clone(&block)));
    });
    RequestHeaders::Shared(block)
}

fn lookup(key: &HeaderKey<'_>) -> Option<Arc<HeaderBlock>> {
    CACHE.with_borrow_mut(|cache| {
        let index = cache.iter().position(|(cached, _)| cached.matches(key))?;
        if index > 0 {
            cache[..=index].rotate_right(1);
        }
        cache.first().map(|(_, block)| Arc::clone(block))
    })
}
