//! Streaming response body.
//!
//! Wraps an mpsc receiver the transport layer pumps response chunks into.
//! Drivers deliver headers as soon as they arrive and push body frames
//! through the channel. The consumer drives [`BodyStream`] as a
//! `futures_util::Stream`.

use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::Stream;
use tokio::sync::mpsc;

/// A streaming response body. Yields `io::Result<Bytes>` chunks as they
/// arrive from the transport.
///
/// Obtained from [`Response::into_stream`](crate::Response::into_stream)
/// after opting in with
/// [`RequestBuilder::stream`](crate::RequestBuilder::stream).
pub struct BodyStream {
    rx: mpsc::Receiver<std::io::Result<Bytes>>,
}

impl BodyStream {
    pub(crate) fn new(rx: mpsc::Receiver<std::io::Result<Bytes>>) -> Self {
        Self { rx }
    }

    /// Build a streaming body from a fully-buffered `Bytes` buffer.
    ///
    /// Yields a single chunk then ends. Used by paths that opted into
    /// streaming but ran on a transport (H1 default, H3) that buffered
    /// the body anyway, so the caller API stays uniform.
    pub(crate) fn from_bytes(buf: Bytes) -> Self {
        let (tx, rx) = mpsc::channel(1);
        if !buf.is_empty() {
            let _ = tx.try_send(Ok(buf));
        }
        drop(tx);
        Self { rx }
    }
}

impl Stream for BodyStream {
    type Item = std::io::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

impl std::fmt::Debug for BodyStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BodyStream").finish_non_exhaustive()
    }
}
