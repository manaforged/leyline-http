use std::borrow::Borrow;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;

use bytes::Bytes;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct HeaderStr(Bytes);

impl Hash for HeaderStr {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl HeaderStr {
    pub fn from_static(s: &'static str) -> Self {
        Self(Bytes::from_static(s.as_bytes()))
    }

    pub(crate) fn from_bytes_lossy(bytes: Bytes) -> Self {
        match std::str::from_utf8(&bytes) {
            Ok(_) => Self(bytes),
            Err(_) => Self(Bytes::from(String::from_utf8_lossy(&bytes).into_owned())),
        }
    }

    pub fn from_utf8(bytes: Bytes) -> Result<Self, std::str::Utf8Error> {
        std::str::from_utf8(&bytes)?;
        Ok(Self(bytes))
    }

    pub fn as_str(&self) -> &str {
        // SAFETY: every constructor validates (or is given pre-validated) UTF-8, so the bytes are always a valid `str`.
        unsafe { std::str::from_utf8_unchecked(&self.0) }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for HeaderStr {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl From<HeaderStr> for Bytes {
    fn from(s: HeaderStr) -> Self {
        s.0
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
mod tests;
