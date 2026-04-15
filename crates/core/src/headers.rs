//! Ordered HTTP header list.

/// Ordered HTTP headers with duplicate-name support.
///
/// This is intentionally not a map: browsers and fingerprinting endpoints can
/// observe header order, and response headers such as `Set-Cookie` are valid
/// multiple times.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeaderList {
    inner: Vec<(String, String)>,
}

impl HeaderList {
    /// Create an empty header list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a header list from ordered pairs.
    pub fn from_pairs(headers: Vec<(String, String)>) -> Self {
        Self { inner: headers }
    }

    /// Return true when the list has no headers.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Append a header without removing existing headers with the same name.
    pub fn append(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.inner.push((name.into(), value.into()));
    }

    /// Set a header, replacing existing values with the same name.
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        self.remove_all(&name);
        self.inner.push((name, value.into()));
    }

    /// Remove all headers with this name.
    pub fn remove_all(&mut self, name: &str) {
        self.inner.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
    }

    /// Return true if a header name is present.
    pub fn contains_name(&self, name: &str) -> bool {
        self.inner.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
    }

    /// Iterate over ordered headers.
    pub fn iter(&self) -> impl Iterator<Item = &(String, String)> {
        self.inner.iter()
    }

    /// Consume into ordered pairs.
    pub fn into_vec(self) -> Vec<(String, String)> {
        self.inner
    }
}

impl From<Vec<(String, String)>> for HeaderList {
    fn from(headers: Vec<(String, String)>) -> Self {
        Self::from_pairs(headers)
    }
}
