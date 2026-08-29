//! Request body — buffered bytes or a streaming source.
//!
//! The default request body shape is a `Vec<u8>` / `Bytes` buffer; this
//! module adds a [`Body`] enum that also carries a `Stream` variant for
//! uploads that should not be materialised in memory (large files, SSE,
//! chunked producers).
//!
//! Existing callers keep working unchanged: `.body(vec![..])`,
//! `.json(&v)`, `.form([...])` all produce buffered bodies via the
//! `From<Vec<u8>> for Body` impl.

use std::fmt;
use std::io;
use std::pin::Pin;

use bytes::Bytes;
use futures_util::Stream;

/// An HTTP request body.
///
/// The `Bytes` variant is the fast path for small buffered payloads and is
/// produced transparently by `RequestBuilder::json` / `form` / `body`.
/// The `Stream` variant carries a `Stream<Item = io::Result<Bytes>>` for
/// payloads that should be fed to the wire incrementally — file uploads,
/// SSE, or any chunked producer.
#[derive(Default)]
#[non_exhaustive]
pub enum Body {
    /// No body (e.g. a GET request).
    #[default]
    Empty,
    /// A fully-materialised byte buffer.
    Bytes(Bytes),
    /// A streaming body. `length_hint` carries the exact content-length
    /// when known — when `Some`, a `content-length` header is sent; when
    /// `None`, HTTP/1.1 uses `Transfer-Encoding: chunked` and HTTP/2 /
    /// HTTP/3 rely on their native framing.
    Stream {
        /// The underlying `Stream` that yields body chunks on poll.
        stream: Pin<Box<dyn Stream<Item = io::Result<Bytes>> + Send + 'static>>,
        /// Known exact content-length, if any.
        length_hint: Option<u64>,
    },
}

impl fmt::Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.debug_struct("Body::Empty").finish(),
            Self::Bytes(b) => f
                .debug_struct("Body::Bytes")
                .field("len", &b.len())
                .finish(),
            Self::Stream { length_hint, .. } => f
                .debug_struct("Body::Stream")
                .field("length_hint", length_hint)
                .finish(),
        }
    }
}

impl Body {
    /// Wrap a `Stream` yielding `io::Result<Bytes>` as a streaming body
    /// with no known length. Transport chooses chunked (H1) or native
    /// framing (H2/H3) accordingly.
    pub fn stream<S>(stream: S) -> Self
    where
        S: Stream<Item = io::Result<Bytes>> + Send + 'static,
    {
        Body::Stream {
            stream: Box::pin(stream),
            length_hint: None,
        }
    }

    /// Wrap a `Stream` with a known exact content length in bytes. The
    /// transport will set `content-length: <length>`.
    pub fn stream_with_length<S>(stream: S, length: u64) -> Self
    where
        S: Stream<Item = io::Result<Bytes>> + Send + 'static,
    {
        Body::Stream {
            stream: Box::pin(stream),
            length_hint: Some(length),
        }
    }

    /// Length hint in bytes, if the body is buffered or a length-known
    /// stream. `None` for a stream with unknown length.
    pub fn len_hint(&self) -> Option<u64> {
        match self {
            Body::Empty => Some(0),
            Body::Bytes(b) => Some(b.len() as u64),
            Body::Stream { length_hint, .. } => *length_hint,
        }
    }

    /// True if this body is empty (either the `Empty` variant or a
    /// zero-length buffered `Bytes`).
    pub fn is_empty(&self) -> bool {
        match self {
            Body::Empty => true,
            Body::Bytes(b) => b.is_empty(),
            Body::Stream { .. } => false,
        }
    }

    /// True if this body is a streaming source. Used internally to pick
    /// the dispatch path in the transport layer.
    pub fn is_stream(&self) -> bool {
        matches!(self, Body::Stream { .. })
    }
}

impl From<Vec<u8>> for Body {
    fn from(v: Vec<u8>) -> Self {
        if v.is_empty() {
            Body::Empty
        } else {
            Body::Bytes(Bytes::from(v))
        }
    }
}

impl From<&'static [u8]> for Body {
    fn from(v: &'static [u8]) -> Self {
        if v.is_empty() {
            Body::Empty
        } else {
            Body::Bytes(Bytes::from_static(v))
        }
    }
}

impl From<String> for Body {
    fn from(s: String) -> Self {
        if s.is_empty() {
            Body::Empty
        } else {
            Body::Bytes(Bytes::from(s.into_bytes()))
        }
    }
}

impl From<&'static str> for Body {
    fn from(s: &'static str) -> Self {
        if s.is_empty() {
            Body::Empty
        } else {
            Body::Bytes(Bytes::from_static(s.as_bytes()))
        }
    }
}

impl From<Bytes> for Body {
    fn from(b: Bytes) -> Self {
        if b.is_empty() {
            Body::Empty
        } else {
            Body::Bytes(b)
        }
    }
}

impl From<()> for Body {
    fn from(_: ()) -> Self {
        Body::Empty
    }
}

#[cfg(test)]
mod tests;
