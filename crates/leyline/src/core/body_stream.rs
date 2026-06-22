//! Streaming response body.
//!
//! Wraps an mpsc receiver the transport layer pumps response chunks into.
//! Drivers deliver headers as soon as they arrive and push body frames
//! through the channel. The consumer drives [`BodyStream`] as a
//! `futures_util::Stream`.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use futures_util::Stream;
use tokio::sync::mpsc;
use tokio::time::Sleep;

/// A streaming response body. Yields `io::Result<Bytes>` chunks as they
/// arrive from the transport.
///
/// Obtained from [`Response::into_stream`](crate::Response::into_stream)
/// after opting in with
/// [`RequestBuilder::stream`](crate::RequestBuilder::stream).
///
/// When a session `read_timeout` is configured, it applies here as a
/// per-chunk idle timeout: if the next body chunk does not arrive within the
/// timeout the stream yields an [`io::ErrorKind::TimedOut`] error. The clock
/// resets after every chunk, so a steady (if slow) stream never trips it —
/// this catches a stalled connection that a request-wide timeout would only
/// notice much later. The same chokepoint covers every transport (H1/H2/H3),
/// since they all deliver through this channel.
///
/// [`io::ErrorKind::TimedOut`]: std::io::ErrorKind::TimedOut
pub struct BodyStream {
    rx: mpsc::Receiver<std::io::Result<Bytes>>,
    /// Per-chunk idle timeout, from the session `read_timeout`.
    read_timeout: Option<Duration>,
    /// Armed while waiting for the next chunk; reset to `None` after each
    /// chunk so the timeout measures the gap between chunks, not total time.
    idle: Option<Pin<Box<Sleep>>>,
}

impl BodyStream {
    pub(crate) fn new(rx: mpsc::Receiver<std::io::Result<Bytes>>) -> Self {
        Self {
            rx,
            read_timeout: None,
            idle: None,
        }
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
        Self::new(rx)
    }

    /// Apply a per-chunk idle read timeout (from the session `read_timeout`).
    pub(crate) fn set_read_timeout(&mut self, timeout: Option<Duration>) {
        self.read_timeout = timeout;
    }
}

impl Stream for BodyStream {
    type Item = std::io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        match this.rx.poll_recv(cx) {
            Poll::Ready(item) => {
                // Progress (chunk, error, or end) — disarm so the next gap
                // gets a fresh timeout.
                this.idle = None;
                Poll::Ready(item)
            }
            Poll::Pending => {
                let Some(timeout) = this.read_timeout else {
                    return Poll::Pending;
                };
                let sleep = this
                    .idle
                    .get_or_insert_with(|| Box::pin(tokio::time::sleep(timeout)));
                match sleep.as_mut().poll(cx) {
                    Poll::Ready(()) => {
                        this.idle = None;
                        Poll::Ready(Some(Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "read timeout: no response body chunk within the configured read_timeout",
                        ))))
                    }
                    Poll::Pending => Poll::Pending,
                }
            }
        }
    }
}

impl std::fmt::Debug for BodyStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BodyStream").finish_non_exhaustive()
    }
}
