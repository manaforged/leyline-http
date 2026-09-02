//! Ordered HTTP header list.

use http::{HeaderName, HeaderValue};

use crate::core::error::{Error, Kind, Result};
use crate::profile::HeaderAnchor;
use crate::profile::preset::HeaderPair;

/// Convert a caller-supplied header name, reporting an invalid name as a builder error.
pub(crate) fn name(n: impl TryInto<HeaderName>) -> Result<HeaderName> {
    n.try_into()
        .map_err(|_| Error::new(Kind::Request).with_message("invalid header name"))
}

/// Convert a caller-supplied header value, reporting an invalid value as a builder error.
pub(crate) fn value(v: impl TryInto<HeaderValue>) -> Result<HeaderValue> {
    v.try_into()
        .map_err(|_| Error::new(Kind::Request).with_message("invalid header value"))
}

/// A single header entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HeaderEntry {
    pub name: HeaderName,
    pub value: HeaderValue,
    pub anchor: Option<HeaderAnchor>,
}

/// Ordered HTTP headers with duplicate-name support.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeaderList {
    inner: Vec<HeaderEntry>,
}

impl HeaderList {
    /// Create an empty header list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a header list from ordered pairs (no anchors).
    pub fn from_pairs<N, V>(headers: Vec<(N, V)>) -> Result<Self>
    where
        N: TryInto<HeaderName>,
        V: TryInto<HeaderValue>,
    {
        let mut list = Self::new();
        for (n, v) in headers {
            list.append(n, v)?;
        }
        Ok(list)
    }

    /// Return true when the list has no headers.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Append a header without removing existing headers with the same name.
    pub fn append(
        &mut self,
        n: impl TryInto<HeaderName>,
        v: impl TryInto<HeaderValue>,
    ) -> Result<()> {
        self.inner.push(HeaderEntry {
            name: name(n)?,
            value: value(v)?,
            anchor: None,
        });
        Ok(())
    }

    /// Set a header, replacing existing values with the same name.
    pub fn set(&mut self, n: impl TryInto<HeaderName>, v: impl TryInto<HeaderValue>) -> Result<()> {
        let name = name(n)?;
        let value = value(v)?;
        self.remove_all(name.as_str());
        self.inner.push(HeaderEntry {
            name,
            value,
            anchor: None,
        });
        Ok(())
    }

    /// Append an anchored header.
    pub fn append_anchored(
        &mut self,
        anchor: HeaderAnchor,
        n: impl TryInto<HeaderName>,
        v: impl TryInto<HeaderValue>,
    ) -> Result<()> {
        self.inner.push(HeaderEntry {
            name: name(n)?,
            value: value(v)?,
            anchor: Some(anchor),
        });
        Ok(())
    }

    /// Remove all headers with this name.
    pub fn remove_all(&mut self, name: &str) {
        self.inner
            .retain(|entry| !entry.name.as_str().eq_ignore_ascii_case(name));
    }

    /// First value for this name, if present.
    pub fn get(&self, name: &str) -> Option<&HeaderValue> {
        self.inner
            .iter()
            .find(|entry| entry.name.as_str().eq_ignore_ascii_case(name))
            .map(|entry| &entry.value)
    }

    /// Iterate over ordered (name, value) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&HeaderName, &HeaderValue)> {
        self.inner.iter().map(|entry| (&entry.name, &entry.value))
    }

    /// Iterate over entries with anchor metadata (internal use).
    pub(crate) fn entries(&self) -> impl Iterator<Item = &HeaderEntry> {
        self.inner.iter()
    }
}

impl<N, V> TryFrom<Vec<(N, V)>> for HeaderList
where
    N: TryInto<HeaderName>,
    V: TryInto<HeaderValue>,
{
    type Error = Error;

    fn try_from(headers: Vec<(N, V)>) -> Result<Self> {
        Self::from_pairs(headers)
    }
}

#[cfg(test)]
#[path = "headers_tests.rs"]
mod tests;

/// Stable-sort headers so those named in `order` come first, in that order; unnamed headers keep their relative order after them.
pub(crate) fn reorder(headers: &mut Vec<HeaderPair>, order: &[String]) {
    let lc_order: Vec<String> = order.iter().map(|s| s.to_ascii_lowercase()).collect();
    let mut buckets: Vec<Vec<HeaderPair>> = vec![Vec::new(); lc_order.len()];
    let mut tail: Vec<HeaderPair> = Vec::new();
    for h in std::mem::take(headers) {
        let lc = h.0.to_ascii_lowercase();
        match lc_order.iter().position(|n| *n == lc) {
            Some(idx) => buckets[idx].push(h),
            None => tail.push(h),
        }
    }
    for b in buckets {
        headers.extend(b);
    }
    headers.extend(tail);
}
