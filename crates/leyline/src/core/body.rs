use std::fmt;
use std::io;
use std::pin::Pin;

use bytes::Bytes;
use futures_util::Stream;

#[derive(Default)]
#[non_exhaustive]
pub enum Body {
    #[default]
    Empty,
    Bytes(Bytes),
    Stream {
        stream: Pin<Box<dyn Stream<Item = io::Result<Bytes>> + Send + 'static>>,
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
    pub fn stream<S>(stream: S) -> Self
    where
        S: Stream<Item = io::Result<Bytes>> + Send + 'static,
    {
        Body::Stream {
            stream: Box::pin(stream),
            length_hint: None,
        }
    }

    pub fn stream_with_length<S>(stream: S, length: u64) -> Self
    where
        S: Stream<Item = io::Result<Bytes>> + Send + 'static,
    {
        Body::Stream {
            stream: Box::pin(stream),
            length_hint: Some(length),
        }
    }

    pub fn len_hint(&self) -> Option<u64> {
        match self {
            Body::Empty => Some(0),
            Body::Bytes(b) => Some(b.len() as u64),
            Body::Stream { length_hint, .. } => *length_hint,
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Body::Empty => true,
            Body::Bytes(b) => b.is_empty(),
            Body::Stream { .. } => false,
        }
    }

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
