use std::io;
use std::pin::Pin;

use bytes::Bytes;
use tokio::sync::mpsc;

use crate::header_str::HeaderStr;

pub enum RequestBody {
    None,
    Buffered(Bytes),
    Streaming {
        stream: Pin<Box<dyn futures_util::Stream<Item = io::Result<Bytes>> + Send + 'static>>,
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

#[derive(Debug)]
pub struct H2ResponseEx {
    pub status: u16,
    pub headers: Vec<(HeaderStr, HeaderStr)>,
    pub body: ResponseBody,
    pub trailers: Option<Vec<(HeaderStr, HeaderStr)>>,
}

pub enum ResponseBody {
    Buffered(Vec<u8>),
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
