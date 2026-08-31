//! Streaming response body.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use futures_util::Stream;
use tokio::sync::mpsc;
use tokio::time::Sleep;

/// A streaming response body.
pub struct BodyStream {
    rx: mpsc::Receiver<std::io::Result<Bytes>>,
    /// Per-chunk idle timeout, from the session `read_timeout`.
    read_timeout: Option<Duration>,
    /// Armed while waiting for the next chunk; reset to `None` after each chunk so the timeout measures the gap between chunks, not total time.
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
