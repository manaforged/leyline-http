use http::{HeaderName, HeaderValue};

use crate::core::error::{Error, Kind, Result};
use crate::profile::HeaderAnchor;
use crate::profile::preset::HeaderPair;

pub(crate) fn name(n: impl TryInto<HeaderName>) -> Result<HeaderName> {
    n.try_into()
        .map_err(|_| Error::new(Kind::Request).with_message("invalid header name"))
}

pub(crate) fn value(v: impl TryInto<HeaderValue>) -> Result<HeaderValue> {
    v.try_into()
        .map_err(|_| Error::new(Kind::Request).with_message("invalid header value"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HeaderEntry {
    pub name: HeaderName,
    pub value: HeaderValue,
    pub anchor: Option<HeaderAnchor>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeaderList {
    inner: Vec<HeaderEntry>,
}

impl HeaderList {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn from_pairs<N, V>(headers: Vec<(N, V)>) -> Result<Self>
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

    pub(crate) fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

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

    pub(crate) fn append_anchored(
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

    pub fn remove_all(&mut self, name: &str) {
        self.inner
            .retain(|entry| !entry.name.as_str().eq_ignore_ascii_case(name));
    }

    pub fn get(&self, name: &str) -> Option<&HeaderValue> {
        self.inner
            .iter()
            .find(|entry| entry.name.as_str().eq_ignore_ascii_case(name))
            .map(|entry| &entry.value)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&HeaderName, &HeaderValue)> {
        self.inner.iter().map(|entry| (&entry.name, &entry.value))
    }

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
