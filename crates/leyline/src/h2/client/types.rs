//! Public request/response body types exchanged with the HTTP/2 client.

use std::io;
use std::pin::Pin;

use bytes::Bytes;
use tokio::sync::mpsc;

use crate::header_str::HeaderStr;

/// Request body supplied to [`super::H2Client::send_request_ex`].
pub enum RequestBody {
    /// No body — headers carry END_STREAM.
    None,
    /// Fully-materialised bytes.
    Buffered(Bytes),
    /// Streaming body: chunks are pulled as they arrive from the caller-provided stream.
    Streaming {
        /// The stream yielding chunks.
        stream: Pin<Box<dyn futures_util::Stream<Item = io::Result<Bytes>> + Send + 'static>>,
        /// Known exact length in bytes, if any.
        length_hint: Option<u64>,
    },
}

impl std::fmt::Debug for RequestBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => f.debug_struct("RequestBody::None").finish(),
            Self::Buffered(b) => f
                .debug_struct("RequestBody::Buffered")
                .field("len", &b.len())
                .finish(),
            Self::Streaming { length_hint, .. } => f
                .debug_struct("RequestBody::Streaming")
                .field("length_hint", length_hint)
                .finish(),
        }
    }
}

impl From<Option<Bytes>> for RequestBody {
    fn from(b: Option<Bytes>) -> Self {
        match b {
            None => RequestBody::None,
            Some(b) if b.is_empty() => RequestBody::None,
            Some(b) => RequestBody::Buffered(b),
        }
    }
}

/// Extended response returned by [`super::H2Client::send_request_ex`].
#[derive(Debug)]
pub struct H2ResponseEx {
    /// HTTP status code.
    pub status: u16,
    /// Response headers in wire order.
    pub headers: Vec<(HeaderStr, HeaderStr)>,
    /// Response body — buffered or streaming.
    pub body: ResponseBody,
    /// Trailers, if any.
    pub trailers: Option<Vec<(HeaderStr, HeaderStr)>>,
}

/// Response body shape delivered alongside an [`H2ResponseEx`].
pub enum ResponseBody {
    /// Fully buffered body (the default).
    Buffered(Vec<u8>),
    /// Streaming body: the caller drains chunks via the receiver.
    Streaming(mpsc::Receiver<io::Result<Bytes>>),
}

impl std::fmt::Debug for ResponseBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Buffered(b) => f
                .debug_struct("ResponseBody::Buffered")
                .field("len", &b.len())
                .finish(),
            Self::Streaming(_) => f
                .debug_struct("ResponseBody::Streaming")
                .finish_non_exhaustive(),
        }
    }
}
