use std::ops::Deref;
use std::sync::{Arc, OnceLock};

use super::HeaderPair;

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
                .get_or_init(|| super::h1::field_lines(&block.pairs))
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

impl HeaderBlock {
    pub(crate) fn new(pairs: Vec<HeaderPair>) -> Self {
        Self {
            pairs,
            h1: OnceLock::new(),
        }
    }
}
