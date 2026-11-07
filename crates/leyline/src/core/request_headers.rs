use std::cell::{Cell, RefCell};
use std::ops::Deref;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use crate::FetchSite;
use crate::profile::RequestKind;
use crate::profile::preset::HeaderPair;

/// A request header list, owned by one request or shared from the header cache.
#[derive(Clone)]
pub(crate) enum RequestHeaders {
    Owned(Vec<HeaderPair>),
    Shared(Arc<HeaderBlock>),
}

/// A cached header list with its HTTP/1.1 field lines, encoded on first use.
pub(crate) struct HeaderBlock {
    pairs: Vec<HeaderPair>,
    h1: OnceLock<Option<Vec<u8>>>,
}

impl RequestHeaders {
    /// Returns the list for changes, copying a shared block first.
    pub(crate) fn to_mut(&mut self) -> &mut Vec<HeaderPair> {
        if let Self::Shared(block) = self {
            *self = Self::Owned(block.pairs.clone());
        }
        match self {
            Self::Owned(pairs) => pairs,
            Self::Shared(_) => unreachable!("a shared block was replaced above"),
        }
    }

    /// Returns the list as an owned vector.
    pub(crate) fn into_vec(self) -> Vec<HeaderPair> {
        match self {
            Self::Owned(pairs) => pairs,
            Self::Shared(block) => {
                Arc::try_unwrap(block).map_or_else(|block| block.pairs.clone(), |block| block.pairs)
            }
        }
    }

    /// Returns the encoded HTTP/1.1 field lines of a shared block whose fields are all valid.
    pub(crate) fn h1_lines(&self) -> Option<&[u8]> {
        match self {
            Self::Owned(_) => None,
            Self::Shared(block) => block
                .h1
                .get_or_init(|| crate::pool::h1::field_lines(&block.pairs))
                .as_deref(),
        }
    }
}

impl Deref for RequestHeaders {
    type Target = [HeaderPair];

    fn deref(&self) -> &[HeaderPair] {
        match self {
            Self::Owned(pairs) => pairs,
            Self::Shared(block) => &block.pairs,
        }
    }
}

impl From<Vec<HeaderPair>> for RequestHeaders {
    fn from(pairs: Vec<HeaderPair>) -> Self {
        Self::Owned(pairs)
    }
}

impl<'a> IntoIterator for &'a RequestHeaders {
    type Item = &'a HeaderPair;
    type IntoIter = std::slice::Iter<'a, HeaderPair>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// A session state id that changes whenever the session state is copied.
pub(crate) struct HeaderId(u64);

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

impl HeaderId {
    /// Returns the id value.
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
    static COUNTS: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
}

/// Returns the header list for `key` from this thread's cache, building and storing it on a miss.
pub(crate) fn cached(
    key: &HeaderKey<'_>,
    build: impl FnOnce() -> Vec<HeaderPair>,
) -> RequestHeaders {
    if let Some(block) = lookup(key) {
        COUNTS.with(|counts| counts.set((counts.get().0 + 1, counts.get().1)));
        return RequestHeaders::Shared(block);
    }
    COUNTS.with(|counts| counts.set((counts.get().0, counts.get().1 + 1)));
    let block = Arc::new(HeaderBlock {
        pairs: build(),
        h1: OnceLock::new(),
    });
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

/// Returns this thread's header cache hit and miss counts.
pub(crate) fn counts() -> (u64, u64) {
    COUNTS.with(Cell::get)
}
