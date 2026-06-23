//! Cheaply-cloneable, UTF-8 response header string.

use std::borrow::Borrow;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;

use bytes::Bytes;

/// A response header name or value: a `Bytes`-backed string slice.
///
/// HTTP/2 and HTTP/3 responses decode header fields straight out of the
/// HPACK/QPACK tables. An indexed (table-hit) field materializes here by a
/// refcount bump or a `'static` borrow — no heap copy — and a literal field
/// allocates once. Construction validates UTF-8, so [`Deref`] to `str` is
/// infallible and callers keep `&str` ergonomics.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct HeaderStr(Bytes);

impl Hash for HeaderStr {
    /// Hash as `str`, not as `[u8]`: `<str>::hash` and `<[u8]>::hash` differ
    /// (length prefix vs `0xff` terminator), so hashing the inner `Bytes`
    /// directly would break the [`Borrow`]`<str>` contract — a `HeaderStr`
    /// map key and a `&str` lookup must hash identically.
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl HeaderStr {
    /// Borrow a `'static` string with no allocation.
    pub fn from_static(s: &'static str) -> Self {
        Self(Bytes::from_static(s.as_bytes()))
    }

    /// Wrap bytes whose UTF-8 validity the caller guarantees — the
    /// HPACK/QPACK decoder validates each field at decode time. Debug
    /// builds assert the invariant.
    pub(crate) fn from_utf8_unchecked(bytes: Bytes) -> Self {
        debug_assert!(std::str::from_utf8(&bytes).is_ok());
        Self(bytes)
    }

    /// Wrap bytes, validating UTF-8.
    pub fn from_utf8(bytes: Bytes) -> Result<Self, std::str::Utf8Error> {
        std::str::from_utf8(&bytes)?;
        Ok(Self(bytes))
    }

    /// Borrow as `&str`. Infallible: every constructor upholds UTF-8.
    pub fn as_str(&self) -> &str {
        // SAFETY: every constructor validates (or is given pre-validated)
        // UTF-8, so the bytes are always a valid `str`.
        unsafe { std::str::from_utf8_unchecked(&self.0) }
    }

    /// The underlying bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl Deref for HeaderStr {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for HeaderStr {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for HeaderStr {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<String> for HeaderStr {
    /// Zero-copy: takes ownership of the `String`'s buffer.
    fn from(s: String) -> Self {
        Self(Bytes::from(s))
    }
}

impl From<&str> for HeaderStr {
    fn from(s: &str) -> Self {
        Self(Bytes::copy_from_slice(s.as_bytes()))
    }
}

impl PartialEq<str> for HeaderStr {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for HeaderStr {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<String> for HeaderStr {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other.as_str()
    }
}

impl fmt::Debug for HeaderStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for HeaderStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_is_zero_copy_and_reads_back() {
        let h = HeaderStr::from_static("content-type");
        assert_eq!(h, "content-type");
        assert_eq!(h.as_str(), "content-type");
        assert!(h.eq_ignore_ascii_case("Content-Type"));
    }

    #[test]
    fn from_string_takes_buffer() {
        let h = HeaderStr::from("text/html".to_string());
        assert_eq!(h.as_bytes(), b"text/html");
    }

    #[test]
    fn rejects_non_utf8() {
        assert!(HeaderStr::from_utf8(Bytes::from_static(&[0xff, 0xfe])).is_err());
    }

    #[test]
    fn deref_enables_str_methods() {
        let h = HeaderStr::from_static("a=1; Path=/; Secure");
        assert_eq!(h.find('='), Some(1));
        assert_eq!(&h[..1], "a");
    }

    #[test]
    fn borrow_str_hash_contract_holds() {
        // A `HeaderStr` map key must be findable by `&str` lookup — only true
        // if `HeaderStr` and its borrowed `str` hash identically (they would
        // not if `Hash` were derived over the inner `[u8]`).
        let mut m = std::collections::HashMap::new();
        m.insert(HeaderStr::from("content-type".to_string()), 1);
        assert_eq!(m.get("content-type"), Some(&1));
        assert_eq!(m.get("absent"), None);
    }
}
