//! Ordered HTTP header list.

use crate::profile::HeaderAnchor;

/// A single header entry. The optional `anchor` records where the
/// caller wants this header spliced into a preset-built list; plain
/// entries (no anchor) keep the previous "append at the end" semantics
/// unless `infer_anchor` finds a well-known Chrome slot for the name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HeaderEntry {
    pub name: String,
    pub value: String,
    pub anchor: Option<HeaderAnchor>,
}

/// Ordered HTTP headers with duplicate-name support.
///
/// This is intentionally not a map: browsers and fingerprinting endpoints can
/// observe header order, and response headers such as `Set-Cookie` are valid
/// multiple times.
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
    pub fn from_pairs(headers: Vec<(String, String)>) -> Self {
        Self {
            inner: headers
                .into_iter()
                .map(|(name, value)| HeaderEntry {
                    name,
                    value,
                    anchor: None,
                })
                .collect(),
        }
    }

    /// Return true when the list has no headers.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Append a header without removing existing headers with the same name.
    pub fn append(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.inner.push(HeaderEntry {
            name: name.into(),
            value: value.into(),
            anchor: None,
        });
    }

    /// Set a header, replacing existing values with the same name.
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        self.remove_all(&name);
        self.inner.push(HeaderEntry {
            name,
            value: value.into(),
            anchor: None,
        });
    }

    /// Append an anchored header. The anchor tells the session where
    /// this header should be spliced into the preset-built header
    /// list — see [`crate::profile::HeaderAnchor`].
    pub fn append_anchored(
        &mut self,
        anchor: HeaderAnchor,
        name: impl Into<String>,
        value: impl Into<String>,
    ) {
        self.inner.push(HeaderEntry {
            name: name.into(),
            value: value.into(),
            anchor: Some(anchor),
        });
    }

    /// Remove all headers with this name.
    pub fn remove_all(&mut self, name: &str) {
        self.inner
            .retain(|entry| !entry.name.eq_ignore_ascii_case(name));
    }

    /// Iterate over ordered (name, value) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.inner.iter().map(|entry| (&entry.name, &entry.value))
    }

    /// Iterate over entries with anchor metadata (internal use).
    pub(crate) fn entries(&self) -> impl Iterator<Item = &HeaderEntry> {
        self.inner.iter()
    }
}

impl From<Vec<(String, String)>> for HeaderList {
    fn from(headers: Vec<(String, String)>) -> Self {
        Self::from_pairs(headers)
    }
}
