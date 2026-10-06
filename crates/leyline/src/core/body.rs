use std::fmt;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::Stream;

pub(crate) type BoxedStream = Pin<Box<dyn Stream<Item = io::Result<Bytes>> + Send + 'static>>;

#[derive(Default)]
pub(crate) enum BodyKind {
    #[default]
    Empty,
    Bytes(Bytes),
    Stream {
        stream: BoxedStream,
        length_hint: Option<u64>,
    },
}

#[derive(Default)]
pub struct Body(pub(crate) BodyKind);

impl fmt::Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = f.debug_struct("Body");
        match &self.0 {
            BodyKind::Empty => out.field("kind", &"empty"),
            BodyKind::Bytes(b) => out.field("kind", &"bytes").field("len", &b.len()),
            BodyKind::Stream { length_hint, .. } => out
                .field("kind", &"stream")
                .field("length_hint", length_hint),
        }
        .finish()
    }
}

impl Stream for Body {
    type Item = io::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match &mut self.0 {
            BodyKind::Empty => Poll::Ready(None),
            BodyKind::Bytes(_) => match std::mem::take(&mut self.0) {
                BodyKind::Bytes(b) => Poll::Ready(Some(Ok(b))),
                _ => Poll::Ready(None),
            },
            BodyKind::Stream { stream, .. } => stream.as_mut().poll_next(cx),
        }
    }
}

const EXCEEDED_DECLARED: &str = "streaming body exceeded declared content-length";
const ENDED_BEFORE_DECLARED: &str = "streaming body ended before declared content-length";

struct Declared {
    inner: BoxedStream,
    declared: u64,
    sent: u64,
    done: bool,
}

impl Declared {
    fn fail(&mut self, message: String) -> Poll<Option<io::Result<Bytes>>> {
        self.done = true;
        Poll::Ready(Some(Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            message,
        ))))
    }
}

impl Stream for Declared {
    type Item = io::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.done {
            return Poll::Ready(None);
        }
        match self.inner.as_mut().poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Ok(chunk))) => {
                let sent = self.sent.saturating_add(chunk.len() as u64);
                if sent > self.declared {
                    return self.fail(EXCEEDED_DECLARED.to_owned());
                }
                self.sent = sent;
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(Some(Err(err))) => {
                self.done = true;
                Poll::Ready(Some(Err(err)))
            }
            Poll::Ready(None) if self.sent < self.declared => {
                let message = format!("{ENDED_BEFORE_DECLARED} ({}/{})", self.sent, self.declared);
                self.fail(message)
            }
            Poll::Ready(None) => {
                self.done = true;
                Poll::Ready(None)
            }
        }
    }
}

impl Body {
    pub(crate) fn bytes(bytes: Bytes) -> Self {
        Body(BodyKind::Bytes(bytes))
    }

    pub fn stream<S>(stream: S, length: Option<u64>) -> Self
    where
        S: Stream<Item = io::Result<Bytes>> + Send + 'static,
    {
        let stream: BoxedStream = match length {
            Some(declared) => Box::pin(Declared {
                inner: Box::pin(stream),
                declared,
                sent: 0,
                done: false,
            }),
            None => Box::pin(stream),
        };
        Body(BodyKind::Stream {
            stream,
            length_hint: length,
        })
    }

    pub fn len_hint(&self) -> Option<u64> {
        match &self.0 {
            BodyKind::Empty => Some(0),
            BodyKind::Bytes(b) => Some(b.len() as u64),
            BodyKind::Stream { length_hint, .. } => *length_hint,
        }
    }

    #[cfg(feature = "http3")]
    pub(crate) fn is_stream(&self) -> bool {
        matches!(self.0, BodyKind::Stream { .. })
    }

    #[cfg(feature = "http3")]
    pub(crate) fn into_parts(self) -> (Option<Bytes>, Option<BoxedStream>) {
        match self.0 {
            BodyKind::Empty => (None, None),
            BodyKind::Bytes(b) => (Some(b), None),
            BodyKind::Stream { stream, .. } => (None, Some(stream)),
        }
    }

    pub(crate) fn into_h2(self) -> crate::h2::client::RequestBody {
        use crate::h2::client::RequestBody;
        match self.0 {
            BodyKind::Empty => RequestBody::None,
            BodyKind::Bytes(b) => RequestBody::Buffered(b),
            BodyKind::Stream {
                stream,
                length_hint,
            } => RequestBody::Streaming {
                stream,
                length_hint,
            },
        }
    }

    pub(crate) fn replay(&self) -> Option<Body> {
        match &self.0 {
            BodyKind::Empty => Some(Body::default()),
            BodyKind::Bytes(b) => Some(Body::bytes(b.clone())),
            BodyKind::Stream { .. } => None,
        }
    }
}

impl From<Bytes> for Body {
    fn from(b: Bytes) -> Self {
        if b.is_empty() {
            Body::default()
        } else {
            Body::bytes(b)
        }
    }
}

impl From<Vec<u8>> for Body {
    fn from(v: Vec<u8>) -> Self {
        Body::from(Bytes::from(v))
    }
}

impl From<&'static [u8]> for Body {
    fn from(v: &'static [u8]) -> Self {
        Body::from(Bytes::from_static(v))
    }
}

impl From<String> for Body {
    fn from(s: String) -> Self {
        Body::from(Bytes::from(s.into_bytes()))
    }
}

impl From<&'static str> for Body {
    fn from(s: &'static str) -> Self {
        Body::from(Bytes::from_static(s.as_bytes()))
    }
}

impl From<()> for Body {
    fn from(_: ()) -> Self {
        Body::default()
    }
}

#[cfg(test)]
mod tests;
