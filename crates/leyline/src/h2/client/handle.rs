use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use tokio::sync::{mpsc, oneshot};

use crate::h2::error::{ErrorCode, H2Error};

#[cfg(feature = "websocket")]
use super::driver::PeerSettingsSnapshot;
use super::driver::{CommandTx, DriverCommand, Head, ResponseSink};
use crate::core::Body;
use crate::core::BodyStream;
use crate::core::ResponseMode;
use crate::core::transport::{TransportBody, TransportResponse};

#[derive(Clone)]
pub struct H2Client {
    pub(super) shared: Arc<Shared>,
}

const _: () = assert!(
    size_of::<H2Client>() == size_of::<usize>(),
    "the pool clones H2Client for every request; keep it one shared pointer"
);

pub(super) struct Shared {
    pub(super) tx: CommandTx,
    pub(super) ping_tx: mpsc::Sender<super::driver::Ping>,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) unanswered_timeout: Arc<AtomicBool>,
    pub(super) streams: Arc<Streams>,
    #[cfg(feature = "websocket")]
    pub(super) peer_settings: Arc<PeerSettingsSnapshot>,
}

impl H2Client {
    /// Returns whether both handles drive the same connection.
    pub(crate) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }

    /// Returns the streams open or reserved on this connection.
    pub fn load(&self) -> usize {
        self.shared.streams.open.load(Ordering::Relaxed)
            + self.shared.streams.reserved.load(Ordering::Relaxed)
    }

    /// Reserves a stream slot while the connection is under its capacity.
    pub fn reserve(&self) -> Option<Slot> {
        let capacity = self.shared.streams.capacity.load(Ordering::Relaxed);
        let open = self.shared.streams.open.load(Ordering::Relaxed);
        self.shared
            .streams
            .reserved
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |reserved| {
                (open + reserved < capacity).then_some(reserved + 1)
            })
            .ok()
            .map(|_| Slot(Arc::clone(&self.shared.streams)))
    }

    pub fn open_streams(&self) -> usize {
        self.shared.streams.open.load(Ordering::Relaxed)
    }

    #[cfg_attr(
        not(feature = "bench-internals"),
        expect(dead_code, reason = "integration tests drive the client directly")
    )]
    pub fn send_shared(
        &self,
        head: Arc<Head>,
        body: Body,
        mode: impl Into<ResponseMode>,
    ) -> impl Future<Output = Result<TransportResponse, H2Error>> {
        self.send_reserved(head, body, mode.into(), None)
    }

    pub(crate) async fn send_reserved(
        &self,
        head: Arc<Head>,
        body: Body,
        mode: ResponseMode,
        slot: Option<Slot>,
    ) -> Result<TransportResponse, H2Error> {
        if self.shared.closed.load(Ordering::Acquire) {
            return Err(H2Error::Stream {
                stream_id: 0,
                code: ErrorCode::RefusedStream,
            });
        }

        let (response_tx, response_rx) = oneshot::channel::<Result<TransportResponse, H2Error>>();
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
        self.shared.tx.send(cmd).await.map_err(|e| {
            tracing::debug!(error = %e, "h2 driver command channel closed");
            H2Error::Stream {
                stream_id: 0,
                code: ErrorCode::RefusedStream,
            }
        })?;

        match response_rx.await {
            Ok(Ok(mut resp)) => {
                if let Some(rx) = stream_body_rx
                    && mode.keeps_stream(resp.status.as_u16())
                {
                    resp.body = TransportBody::Streaming(Box::new(BodyStream::new(rx)));
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

    pub fn is_closed(&self) -> bool {
        self.shared.closed.load(Ordering::Acquire)
    }

    /// Sends a PING ahead of later requests and fails the connection if no ACK arrives by `timeout`.
    pub async fn probe(
        &self,
        timeout: std::time::Duration,
    ) -> Result<oneshot::Receiver<()>, H2Error> {
        if self.shared.closed.load(Ordering::Acquire) {
            return Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "connection closed".into(),
            });
        }
        let (ack_tx, ack_rx) = oneshot::channel();
        let deadline = std::time::Instant::now() + timeout;
        self.shared
            .ping_tx
            .send((ack_tx, deadline))
            .await
            .map_err(|e| H2Error::Connection {
                code: ErrorCode::NoError,
                reason: format!("driver task has exited: {e}"),
            })?;
        Ok(ack_rx)
    }

    pub(crate) fn take_unanswered_timeout(&self) -> bool {
        self.shared.unanswered_timeout.swap(false, Ordering::AcqRel)
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
