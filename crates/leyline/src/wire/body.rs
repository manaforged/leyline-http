use std::fmt;
use std::io;
use std::path::Path;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::Stream;

pub(crate) type BoxedStream =
    Pin<Box<dyn Stream<Item = io::Result<Bytes>> + Send + Sync + 'static>>;

fn exclusive<S>(stream: S) -> BoxedStream
where
    S: Stream<Item = io::Result<Bytes>> + Send + Unpin + 'static,
{
    Box::pin(Exclusive(Mutex::new(stream)))
}

struct Exclusive<S>(Mutex<S>);

impl<S: Stream + Unpin> Stream for Exclusive<S> {
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let stream = crate::util::get_mut(&mut self.get_mut().0);
        Pin::new(stream).poll_next(cx)
    }
}

/// A stream's declared length, stored so `Option<Length>` takes eight bytes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Length(std::num::NonZeroU64);

impl Length {
    fn new(bytes: u64) -> Option<Self> {
        bytes
            .checked_add(1)
            .and_then(std::num::NonZeroU64::new)
            .map(Self)
    }

    pub(crate) fn get(self) -> u64 {
        self.0.get() - 1
    }
}

#[derive(Default)]
pub(crate) enum BodyKind {
    #[default]
    Empty,
    Bytes(Bytes),
    Stream {
        stream: BoxedStream,
        length_hint: Option<Length>,
    },
}

/// An HTTP body, sent or received: empty, in-memory bytes, or a stream.
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
    type Item = crate::error::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match &mut self.0 {
            BodyKind::Empty => Poll::Ready(None),
            BodyKind::Bytes(_) => match std::mem::take(&mut self.0) {
                BodyKind::Bytes(b) => Poll::Ready(Some(Ok(b))),
                _ => Poll::Ready(None),
            },
            BodyKind::Stream { stream, .. } => stream
                .as_mut()
                .poll_next(cx)
                .map(|chunk| chunk.map(|chunk| chunk.map_err(crate::error::Error::from))),
        }
    }
}

const EXCEEDED_DECLARED: &str = "streaming body exceeded declared content-length";
const ENDED_BEFORE_DECLARED: &str = "streaming body ended before declared content-length";

struct Declared {
    inner: Pin<Box<dyn Stream<Item = io::Result<Bytes>> + Send + 'static>>,
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

    /// Wraps a byte stream as a body; `length` is the `content-length` in bytes, which the stream must send exactly or fail, and `None` sends the body chunked with no declared length.
    pub fn stream<S>(stream: S, length: impl Into<Option<u64>>) -> Self
    where
        S: Stream<Item = io::Result<Bytes>> + Send + 'static,
    {
        let length_hint = length.into().and_then(Length::new);
        let stream = Box::pin(stream);
        let stream = match length_hint {
            Some(declared) => exclusive(Declared {
                inner: stream,
                declared: declared.get(),
                sent: 0,
                done: false,
            }),
            None => exclusive(stream),
        };
        Body(BodyKind::Stream {
            stream,
            length_hint,
        })
    }

    /// Streams the file at `path` as a body whose `content-length` is the file size read now, and fails when the file cannot be opened or is not a regular file.
    pub fn file(path: impl AsRef<Path>) -> crate::error::Result<Self> {
        let require_file = |meta: &std::fs::Metadata| -> io::Result<()> {
            if meta.is_file() {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "body path is not a file",
                ))
            }
        };
        let path = path.as_ref();
        require_file(&std::fs::metadata(path)?)?;
        let file = std::fs::File::open(path)?;
        let opened_meta = file.metadata()?;
        require_file(&opened_meta)?;
        let length = opened_meta.len();
        let stream = tokio_util::io::ReaderStream::new(tokio::fs::File::from_std(file));
        Ok(Self::stream(stream, Some(length)))
    }

    /// Returns the bytes of a body held in memory, empty for an empty body, or `None` for a streamed body.
    #[must_use]
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match &self.0 {
            BodyKind::Empty => Some(&[]),
            BodyKind::Bytes(bytes) => Some(&bytes[..]),
            BodyKind::Stream { .. } => None,
        }
    }

    /// Returns the known byte length of the body (`0` when empty, the buffer length in memory, or the declared stream or file length), or `None` for an unsized stream.
    #[must_use]
    pub fn content_length(&self) -> Option<u64> {
        self.len_hint()
    }

    /// Returns a copy of a body held in memory without copying its bytes, or `None` for a streamed body.
    #[must_use]
    pub fn try_clone(&self) -> Option<Self> {
        match &self.0 {
            BodyKind::Empty => Some(Body::default()),
            BodyKind::Bytes(b) => Some(Body::bytes(b.clone())),
            BodyKind::Stream { .. } => None,
        }
    }

    /// Returns the next chunk of the body, an error that ends it, or `None` after the last chunk.
    pub async fn next(&mut self) -> Option<crate::error::Result<Bytes>> {
        futures_util::StreamExt::next(self).await
    }

    pub(crate) fn shared<S>(stream: S) -> Self
    where
        S: Stream<Item = io::Result<Bytes>> + Send + Sync + 'static,
    {
        Body(BodyKind::Stream {
            stream: Box::pin(stream),
            length_hint: None,
        })
    }

    pub(crate) fn len_hint(&self) -> Option<u64> {
        match &self.0 {
            BodyKind::Empty => Some(0),
            BodyKind::Bytes(b) => Some(b.len() as u64),
            BodyKind::Stream { length_hint, .. } => length_hint.map(Length::get),
        }
    }

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
