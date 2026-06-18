//! [`H2ConnectStream`] — a bidirectional byte stream layered over an
//! HTTP/2 connection opened via RFC 8441 extended CONNECT.
//!
//! Self-contained: it talks to the driver only through channels. The
//! struct fields are `pub(super)` so the driver and handle modules can
//! construct it, but they do not leak past the `client` module.

use std::io;
use std::pin::Pin;

use bytes::Bytes;
use tokio::sync::mpsc;
use tokio_util::sync::PollSender;

/// Bidirectional stream over an HTTP/2 connection opened via
/// RFC 8441 extended CONNECT.
///
/// Implements `tokio::io::AsyncRead` and `tokio::io::AsyncWrite` so higher layers
/// (tokio-tungstenite, an arbitrary framed protocol) can run on top.
/// Writes are chunked through the H2 driver's flow-control machinery;
/// reads drain DATA frames the driver pushes into the inbound channel.
/// Dropping the stream closes the write half gracefully with an
/// END_STREAM DATA frame.
pub struct H2ConnectStream {
    pub(super) status: u16,
    pub(super) response_headers: Vec<(String, String)>,
    /// Wrapped in `Option` so `poll_shutdown` and `Drop` can take it
    /// to signal EOF to the driver-side relay task. `PollSender` rather
    /// than a raw `mpsc::Sender` because `poll_write` must keep its
    /// channel reservation — and with it the registered waker — alive
    /// across `Pending` polls; a per-poll `reserve()` future dropped on
    /// `Pending` deregisters the waker and the writer hangs forever.
    pub(super) write_tx: Option<PollSender<io::Result<Bytes>>>,
    pub(super) read_rx: mpsc::Receiver<io::Result<Bytes>>,
    pub(super) read_leftover: Bytes,
    pub(super) read_eof: bool,
    /// Tracks `poll_shutdown` state so a caller awaiting
    /// `AsyncWriteExt::shutdown` blocks until the driver has actually
    /// written the END_STREAM DATA frame to the wire. Without this,
    /// `shutdown()` would resolve before any bytes — much less the
    /// END_STREAM — landed on the wire and the caller racing with a
    /// server that expects clean close could observe the close from
    /// the wrong side.
    pub(super) shutdown_state: ShutdownState,
}

/// State machine for `H2ConnectStream::poll_shutdown`.
pub(super) enum ShutdownState {
    /// Initial state — the caller hasn't started shutdown yet.
    Open,
    /// Caller dropped the sender; now waiting for the driver-side
    /// relay to observe EOF and emit END_STREAM. `read_eof` on the
    /// response half is the observable signal, since the driver
    /// closes the read side's mpsc as part of stream completion.
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
    /// The server's `:status` from the response HEADERS — `200` for a
    /// successful extended CONNECT per RFC 8441 §5.
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
        // `PollSender` holds the channel reservation inside itself, so
        // the registered waker survives a `Pending` return. A per-poll
        // `pin!(tx.reserve())` future is wrong here: dropping it on
        // `Pending` deregisters the waker from the channel's waitlist
        // and the writer is never repolled when capacity frees.
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
        // First call: close the write half. Dropping the sender drives
        // the driver-side relay to EOF, which emits END_STREAM.
        if matches!(self.shutdown_state, ShutdownState::Open) {
            self.write_tx = None;
            self.shutdown_state = ShutdownState::Draining;
        }

        if self.read_eof {
            return Poll::Ready(Ok(()));
        }

        // Drain whatever chunks the driver has queued in a bounded
        // inner loop. The prior implementation pulled one chunk per
        // poll and self-wake'd, which spun CPU at wire rate whenever
        // the peer kept streaming DATA during shutdown. Bounded draw
        // + waker-based repoll keeps the task cooperative.
        for _ in 0..H2_CONNECT_SHUTDOWN_DRAIN_BUDGET {
            match self.read_rx.poll_recv(cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    if bytes.is_empty() {
                        continue;
                    }
                    // Cap on the leftover buffer. Shutdown is not a
                    // licence for the peer to stream unbounded bytes
                    // into our memory. When the cap is hit, surface
                    // an IO error so callers see the truncation
                    // instead of a silent clean-EOF — returning
                    // Ready(Ok(())) here would, for WebSocket or
                    // binary-download workloads, look like a successful
                    // close and silently drop tail bytes.
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
                // The waker is registered with the mpsc; the runtime
                // will repoll us when the next chunk (or close)
                // arrives. No self-wake needed.
                Poll::Pending => return Poll::Pending,
            }
        }
        // We drained our budget without seeing close. Yield back to
        // the runtime and ask to be repolled, so other tasks get a
        // turn instead of us hogging the worker with leftover copies.
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

/// Max bytes we accumulate into `H2ConnectStream::read_leftover` during
/// `poll_shutdown`. When the peer keeps streaming DATA after the caller
/// starts shutdown, tail bytes beyond the cap are dropped — shutdown is
/// not a licence to burn unbounded memory.
const H2_CONNECT_LEFTOVER_CAP: usize = 1 << 20; // 1 MiB

/// Max chunks drained per `poll_shutdown` iteration. Keeps the task
/// cooperative under a peer that bursts many small DATA frames.
const H2_CONNECT_SHUTDOWN_DRAIN_BUDGET: usize = 64;

impl Drop for H2ConnectStream {
    fn drop(&mut self) {
        self.write_tx = None;
    }
}

#[cfg(test)]
mod connect_stream_tests {
    use std::io;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    use bytes::Bytes;
    use tokio::io::AsyncWrite;
    use tokio::sync::mpsc;

    use super::{H2ConnectStream, ShutdownState};

    struct CountWaker(AtomicUsize);

    impl Wake for CountWaker {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    // Lost-wakeup gate: `poll_write` must not build a fresh `reserve()`
    // future on every poll. Doing so means returning `Pending` drops the
    // future and deregisters the waker from the channel's waitlist — the
    // task is never repolled when capacity frees and the write hangs
    // forever under backpressure.
    #[test]
    fn poll_write_backpressure_wakes_when_capacity_frees() {
        let (write_tx, mut write_rx) = mpsc::channel::<io::Result<Bytes>>(1);
        // Fill the only slot so the next reserve must park.
        write_tx.try_send(Ok(Bytes::from_static(b"fill"))).unwrap();
        let (_read_tx, read_rx) = mpsc::channel::<io::Result<Bytes>>(1);

        let mut stream = H2ConnectStream {
            shutdown_state: ShutdownState::Open,
            status: 200,
            response_headers: Vec::new(),
            write_tx: Some(tokio_util::sync::PollSender::new(write_tx)),
            read_rx,
            read_leftover: Bytes::new(),
            read_eof: false,
        };

        let woken = Arc::new(CountWaker(AtomicUsize::new(0)));
        let waker = Waker::from(woken.clone());
        let mut cx = Context::from_waker(&waker);

        assert!(
            Pin::new(&mut stream)
                .poll_write(&mut cx, b"hello")
                .is_pending(),
            "first write must hit backpressure"
        );

        // Free the slot; the channel wakes registered reservers.
        assert!(write_rx.try_recv().is_ok());
        assert!(
            woken.0.load(Ordering::SeqCst) > 0,
            "freeing channel capacity must wake the parked writer"
        );

        // The retried write now completes.
        match Pin::new(&mut stream).poll_write(&mut cx, b"hello") {
            Poll::Ready(Ok(n)) => assert_eq!(n, 5),
            other => panic!("expected Ready(Ok(5)), got {other:?}"),
        }
    }
}
