use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use tokio::sync::{mpsc, oneshot};

use crate::h2::error::{ErrorCode, H2Error};

#[cfg(feature = "websocket")]
use super::driver::PeerSettingsSnapshot;
use super::driver::{DriverCommand, Head, ResponseSink};
use super::types::{H2ResponseEx, RequestBody, ResponseBody};
use crate::core::ResponseMode;

#[derive(Clone)]
pub struct H2Client {
    pub(super) tx: mpsc::Sender<DriverCommand>,
    pub(super) ping_tx: mpsc::Sender<oneshot::Sender<()>>,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) unanswered_timeout: Arc<AtomicBool>,
    pub(super) streams: Arc<Streams>,
    #[cfg(feature = "websocket")]
    pub(super) peer_settings: Arc<PeerSettingsSnapshot>,
}

impl H2Client {
    /// Returns the streams open or reserved on this connection.
    pub fn load(&self) -> usize {
        self.streams.open.load(Ordering::Relaxed) + self.streams.reserved.load(Ordering::Relaxed)
    }

    /// Reserves a stream slot while the connection is under its capacity.
    pub fn reserve(&self) -> Option<Slot> {
        let capacity = self.streams.capacity.load(Ordering::Relaxed);
        let open = self.streams.open.load(Ordering::Relaxed);
        self.streams
            .reserved
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |reserved| {
                (open + reserved < capacity).then_some(reserved + 1)
            })
            .ok()
            .map(|_| Slot(Arc::clone(&self.streams)))
    }

    pub fn open_streams(&self) -> usize {
        self.streams.open.load(Ordering::Relaxed)
    }

    #[cfg_attr(
        not(feature = "bench-internals"),
        expect(dead_code, reason = "integration tests drive the client directly")
    )]
    pub async fn send_shared(
        &self,
        head: Arc<Head>,
        body: RequestBody,
        mode: impl Into<ResponseMode>,
    ) -> Result<H2ResponseEx, H2Error> {
        self.send_reserved(head, body, mode.into(), None).await
    }

    pub(crate) async fn send_reserved(
        &self,
        head: Arc<Head>,
        body: RequestBody,
        mode: ResponseMode,
        slot: Option<Slot>,
    ) -> Result<H2ResponseEx, H2Error> {
        if self.closed.load(Ordering::Acquire) {
            return Err(H2Error::Stream {
                stream_id: 0,
                code: ErrorCode::RefusedStream,
            });
        }

        let (response_tx, response_rx) = oneshot::channel::<Result<H2ResponseEx, H2Error>>();
        let (sink, stream_body_rx) = match mode {
            ResponseMode::Buffered => (ResponseSink::Buffered(response_tx), None),
            ResponseMode::Streamed => {
                let (sink, receiver) = ResponseSink::streaming(response_tx);
                (sink, Some(receiver))
            }
            ResponseMode::ErrorPrefix(_) => {
                let (sink, receiver) = ResponseSink::adaptive(response_tx, mode);
                (sink, Some(receiver))
            }
        };

        let cmd = DriverCommand::SendRequest {
            head,
            body,
            sink,
            slot,
        };
        self.tx.send(cmd).await.map_err(|e| {
            tracing::debug!(error = %e, "h2 driver command channel closed");
            H2Error::Stream {
                stream_id: 0,
                code: ErrorCode::RefusedStream,
            }
        })?;

        match response_rx.await {
            Ok(Ok(mut resp)) => {
                if let Some(rx) = stream_body_rx
                    && mode.keeps_stream(resp.status)
                {
                    resp.body = ResponseBody::Streaming(rx);
                }
                Ok(resp)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "driver dropped response sender".into(),
            }),
        }
    }

    pub async fn ping(&self) -> Result<(), H2Error> {
        if self.closed.load(Ordering::Acquire) {
            return Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "connection closed".into(),
            });
        }
        let (ack_tx, ack_rx) = oneshot::channel();
        self.ping_tx
            .send(ack_tx)
            .await
            .map_err(|e| H2Error::Connection {
                code: ErrorCode::NoError,
                reason: format!("driver task has exited: {e}"),
            })?;
        ack_rx.await.map_err(|e| H2Error::Connection {
            code: ErrorCode::NoError,
            reason: format!("connection closed before the ping was acknowledged: {e}"),
        })
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub(crate) fn take_unanswered_timeout(&self) -> bool {
        self.unanswered_timeout.swap(false, Ordering::AcqRel)
    }
}

/// A stream slot reserved on an [`H2Client`] until its request opens a stream.
pub struct Slot(Arc<Streams>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.reserved.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) struct Streams {
    pub(super) open: AtomicUsize,
    pub(super) reserved: AtomicUsize,
    pub(super) capacity: AtomicUsize,
}

impl Streams {
    pub(super) fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            open: AtomicUsize::new(0),
            reserved: AtomicUsize::new(0),
            capacity: AtomicUsize::new(capacity),
        })
    }
}
