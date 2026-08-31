//! [`H2ConnectStream`] — a bidirectional byte stream layered over an HTTP/2 connection opened via RFC 8441 extended CONNECT.

use std::io;
use std::pin::Pin;

use bytes::Bytes;
use tokio::sync::mpsc;
use tokio_util::sync::PollSender;

/// Bidirectional stream over an HTTP/2 connection opened via RFC 8441 extended CONNECT.
pub struct H2ConnectStream {
    pub(super) status: u16,
    pub(super) response_headers: Vec<(String, String)>,
    /// Wrapped in `Option` so `poll_shutdown` and `Drop` can take it to signal EOF to the driver-side relay task.
    pub(super) write_tx: Option<PollSender<io::Result<Bytes>>>,
    pub(super) read_rx: mpsc::Receiver<io::Result<Bytes>>,
    pub(super) read_leftover: Bytes,
    pub(super) read_eof: bool,
    /// Tracks `poll_shutdown` state so a caller awaiting `AsyncWriteExt::shutdown` blocks until the driver has actually written the END_STREAM DATA frame to the wire.
    pub(super) shutdown_state: ShutdownState,
}

/// State machine for `H2ConnectStream::poll_shutdown`.
pub(super) enum ShutdownState {
    /// Initial state — the caller hasn't started shutdown yet.
    Open,
    /// Caller dropped the sender; now waiting for the driver-side relay to observe EOF and emit END_STREAM.
    Draining,
}

impl std::fmt::Debug for H2ConnectStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("H2ConnectStream")
            .field("status", &self.status)
            .field("response_headers_len", &self.response_headers.len())
            .field("write_open", &self.write_tx.is_some())
            .field("read_eof", &self.read_eof)
            .finish()
    }
}

impl H2ConnectStream {
    /// The server's `:status` from the response HEADERS — `200` for a successful extended CONNECT per RFC 8441 §5.
    pub fn status(&self) -> u16 {
        self.status
    }

    /// Non-pseudo response headers.
    pub fn response_headers(&self) -> &[(String, String)] {
        &self.response_headers
    }
}

impl tokio::io::AsyncRead for H2ConnectStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        use std::task::Poll;
        if !self.read_leftover.is_empty() {
            let take = self.read_leftover.len().min(buf.remaining());
            let chunk = self.read_leftover.slice(0..take);
            buf.put_slice(&chunk);
            self.read_leftover = self.read_leftover.slice(take..);
            return Poll::Ready(Ok(()));
        }
        if self.read_eof {
            return Poll::Ready(Ok(()));
        }
        match self.read_rx.poll_recv(cx) {
            Poll::Ready(Some(Ok(bytes))) => {
                if bytes.is_empty() {
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                let take = bytes.len().min(buf.remaining());
                buf.put_slice(&bytes[..take]);
                if take < bytes.len() {
                    self.read_leftover = bytes.slice(take..);
                }
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Err(e)),
            Poll::Ready(None) => {
                self.read_eof = true;
                Poll::Ready(Ok(()))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl tokio::io::AsyncWrite for H2ConnectStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        use std::task::Poll;
        let this = self.get_mut();
        let tx = match this.write_tx.as_mut() {
            Some(t) => t,
            None => {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "H2ConnectStream write half closed",
                )));
            }
        };
        match tx.poll_reserve(cx) {
            Poll::Ready(Ok(())) => {
                let n = buf.len();
                if tx.send_item(Ok(Bytes::copy_from_slice(buf))).is_err() {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "H2 driver dropped the CONNECT stream",
                    )));
                }
                Poll::Ready(Ok(n))
            }
            Poll::Ready(Err(_closed)) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "H2 driver dropped the CONNECT stream",
            ))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        use std::task::Poll;
        if matches!(self.shutdown_state, ShutdownState::Open) {
            self.write_tx = None;
            self.shutdown_state = ShutdownState::Draining;
        }

        if self.read_eof {
            return Poll::Ready(Ok(()));
        }

        for _ in 0..H2_CONNECT_SHUTDOWN_DRAIN_BUDGET {
            match self.read_rx.poll_recv(cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    if bytes.is_empty() {
                        continue;
                    }
                    if self.read_leftover.len() >= H2_CONNECT_LEFTOVER_CAP {
                        self.read_eof = true;
                        return Poll::Ready(Err(io::Error::other(
                            "H2ConnectStream shutdown drain exceeded \
                             H2_CONNECT_LEFTOVER_CAP; peer streamed past \
                             shutdown faster than the caller drained",
                        )));
                    }
                    if self.read_leftover.is_empty() {
                        self.read_leftover = bytes;
                    } else {
                        let mut buf =
                            bytes::BytesMut::with_capacity(self.read_leftover.len() + bytes.len());
                        buf.extend_from_slice(&self.read_leftover);
                        buf.extend_from_slice(&bytes);
                        self.read_leftover = buf.freeze();
                    }
                }
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Err(e)),
                Poll::Ready(None) => {
                    self.read_eof = true;
                    return Poll::Ready(Ok(()));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

/// Max bytes we accumulate into `H2ConnectStream::read_leftover` during `poll_shutdown`.
const H2_CONNECT_LEFTOVER_CAP: usize = 1 << 20;
/// Max chunks drained per `poll_shutdown` iteration.
const H2_CONNECT_SHUTDOWN_DRAIN_BUDGET: usize = 64;

impl Drop for H2ConnectStream {
    fn drop(&mut self) {
        self.write_tx = None;
    }
}

#[cfg(test)]
mod connect_stream_tests;
