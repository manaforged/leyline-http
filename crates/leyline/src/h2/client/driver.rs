use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Semaphore, mpsc, oneshot};
use tokio::task::AbortHandle;

use crate::core::ResponseMode;
use crate::h2::codec::{FrameReader, FrameWriter};
use crate::h2::config::H2Config;
use crate::h2::connection::{PeerSettings, RstFloodDetector};
use crate::h2::error::{ErrorCode, H2Error};
use crate::h2::frame::*;
use crate::h2::hpack;
use crate::h2::stream_state::{StreamState, StreamStateError};

use crate::core::transport::{ResponseState, TransportBody, TransportResponse};

mod bootstrap;
mod command;
mod event;
mod lifecycle;
mod output;
mod protocol;
mod recv;
mod send;
mod stream_map;

pub use self::bootstrap::start;
pub use protocol::Head;
pub(crate) use protocol::{CommandTx, DriverCommand, checked_window_add};

const COMMAND_CHANNEL_CAPACITY: usize = 1024;

const PING_CHANNEL_CAPACITY: usize = 16;

/// A PING's ACK sender and the time by which the ACK must arrive.
pub(crate) type Ping = (oneshot::Sender<()>, std::time::Instant);

const SWEEP_PERIOD: std::time::Duration = std::time::Duration::from_millis(100);

const FLUSH_PERIOD: std::time::Duration = std::time::Duration::from_millis(1);

pub(super) const STREAM_REQ_BODY_CAPACITY: usize = 32;

pub(super) const STREAM_RESP_BODY_CAPACITY: usize = 32;

#[doc(hidden)]
#[derive(Debug)]
pub struct PeerSettingsSnapshot {
    enable_connect_protocol: std::sync::atomic::AtomicBool,
}

impl PeerSettingsSnapshot {
    fn new() -> Self {
        Self {
            enable_connect_protocol: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn set_enable_connect_protocol(&self, value: bool) {
        self.enable_connect_protocol.store(value, Ordering::Relaxed);
    }

    #[cfg(feature = "websocket")]
    pub fn enable_connect_protocol(&self) -> bool {
        self.enable_connect_protocol.load(Ordering::Relaxed)
    }
}

struct PendingSend {
    remaining: Bytes,
}

pub(crate) enum ResponseSink {
    Buffered(oneshot::Sender<Result<TransportResponse, H2Error>>),
    StreamingEx {
        headers_tx: Option<oneshot::Sender<Result<TransportResponse, H2Error>>>,
        body_tx: mpsc::Sender<io::Result<Bytes>>,
        terminal: mpsc::OwnedPermit<io::Result<Bytes>>,
    },
    Adaptive {
        tx: oneshot::Sender<Result<TransportResponse, H2Error>>,
        body_tx: mpsc::Sender<io::Result<Bytes>>,
        terminal: mpsc::OwnedPermit<io::Result<Bytes>>,
        mode: ResponseMode,
    },
}

impl ResponseSink {
    pub(super) fn is_cancelled(&self) -> bool {
        match self {
            Self::Buffered(tx) | Self::Adaptive { tx, .. } => tx.is_closed(),
            Self::StreamingEx {
                headers_tx,
                body_tx,
                ..
            } => headers_tx.as_ref().is_none_or(oneshot::Sender::is_closed) && body_tx.is_closed(),
        }
    }

    pub(super) fn streaming(
        headers_tx: oneshot::Sender<Result<TransportResponse, H2Error>>,
    ) -> (Self, mpsc::Receiver<io::Result<Bytes>>) {
        let (sink, body_rx) = Self::adaptive(headers_tx, ResponseMode::Streamed);
        (sink.settle(http::StatusCode::OK), body_rx)
    }

    pub(super) fn adaptive(
        tx: oneshot::Sender<Result<TransportResponse, H2Error>>,
        mode: ResponseMode,
    ) -> (Self, mpsc::Receiver<io::Result<Bytes>>) {
        let (body_tx, body_rx) = mpsc::channel(STREAM_RESP_BODY_CAPACITY + 1);
        let terminal = body_tx
            .clone()
            .try_reserve_owned()
            .expect("new body channel has an available slot");
        (
            Self::Adaptive {
                tx,
                body_tx,
                terminal,
                mode,
            },
            body_rx,
        )
    }

    pub(super) fn settle(self, status: http::StatusCode) -> Self {
        match self {
            Self::Adaptive {
                tx,
                body_tx,
                terminal,
                mode,
            } if mode.keeps_stream(status.as_u16()) => Self::StreamingEx {
                headers_tx: Some(tx),
                body_tx,
                terminal,
            },
            Self::Adaptive { tx, .. } => Self::Buffered(tx),
            settled => settled,
        }
    }
}

enum SendBodyInput {
    None,
    Streaming {
        pending_buf: VecDeque<Bytes>,
        closed: bool,
        error: Option<io::Error>,
    },
}

struct StreamActor {
    state: StreamState,
    send_window: i64,
    recv_window: i64,
    response_tx: Option<ResponseSink>,
    status: http::StatusCode,
    response: ResponseState,
    resp_headers: http::HeaderMap,
    body: Vec<u8>,
    trailers: http::HeaderMap,
    drop_body: bool,
    pending_send: Option<PendingSend>,
    send_body_input: SendBodyInput,
    upload_credit: Option<Arc<Semaphore>>,
    pump: Option<AbortHandle>,
    stalled: std::collections::VecDeque<Bytes>,
    remote_done: bool,
    declared_len: Option<u64>,
    recv_len: u64,
}

impl StreamActor {
    fn new(send_window: i64, recv_window: i64, sink: ResponseSink, drop_body: bool) -> Self {
        Self {
            state: StreamState::Idle,
            send_window,
            recv_window,
            response_tx: Some(sink),
            status: http::StatusCode::OK,
            response: ResponseState::Initial(0),
            resp_headers: http::HeaderMap::new(),
            body: Vec::new(),
            trailers: http::HeaderMap::new(),
            drop_body,
            pending_send: None,
            send_body_input: SendBodyInput::None,
            upload_credit: None,
            pump: None,
            stalled: std::collections::VecDeque::new(),
            remote_done: false,
            declared_len: None,
            recv_len: 0,
        }
    }

    fn length_mismatch(&self) -> bool {
        !self.drop_body && self.declared_len.is_some_and(|len| len != self.recv_len)
    }

    fn response(&mut self, body: Vec<u8>) -> TransportResponse {
        TransportResponse {
            status: self.status,
            headers: std::mem::take(&mut self.resp_headers).into(),
            trailers: crate::core::transport::trailers(std::mem::take(&mut self.trailers)),
            body: TransportBody::Buffered(body),
            version: http::Version::HTTP_2,
            tls: None,
            timing: crate::core::ResponseTiming::default(),
        }
    }

    fn deliver_headers_streaming(&mut self) {
        if let Some(ResponseSink::StreamingEx { headers_tx, .. }) = self.response_tx.as_mut()
            && let Some(tx) = headers_tx.take()
        {
            drop(tx.send(Ok(self.response(Vec::new()))));
        }
    }

    fn deliver_ok(&mut self) {
        match self.response_tx.take() {
            Some(ResponseSink::Buffered(tx)) => {
                let body = std::mem::take(&mut self.body);
                drop(tx.send(Ok(self.response(body))));
            }
            Some(ResponseSink::StreamingEx {
                body_tx, terminal, ..
            }) if !self.stalled.is_empty() && !body_tx.is_closed() => {
                let mut tail = BytesMut::with_capacity(self.stalled.iter().map(Bytes::len).sum());
                for chunk in self.stalled.drain(..) {
                    tail.extend_from_slice(&chunk);
                }
                drop(terminal.send(Ok(tail.freeze())));
            }
            None | Some(ResponseSink::StreamingEx { .. } | ResponseSink::Adaptive { .. }) => {}
        }
    }

    fn deliver_err(&mut self, err: H2Error) {
        match self.response_tx.take() {
            Some(ResponseSink::Buffered(tx) | ResponseSink::Adaptive { tx, .. }) => {
                drop(tx.send(Err(err)));
            }
            Some(ResponseSink::StreamingEx {
                headers_tx,
                terminal,
                ..
            }) => {
                if let Some(tx) = headers_tx {
                    drop(tx.send(Err(err)));
                } else {
                    drop(terminal.send(Err(io::Error::other(format!("h2 stream failed: {err}")))));
                }
            }
            None => {}
        }
    }
}

impl StreamActor {
    fn cancel_upload(&mut self) {
        if let Some(pump) = self.pump.take() {
            pump.abort();
        }
        self.pending_send = None;
        self.send_body_input = SendBodyInput::None;
        self.upload_credit = None;
    }
}

impl Drop for StreamActor {
    fn drop(&mut self) {
        self.cancel_upload();
        if self.response_tx.is_some() {
            self.deliver_err(H2Error::Connection {
                code: ErrorCode::InternalError,
                reason: "connection driver stopped before response completion".into(),
            });
        }
    }
}

type BodyChunkIn = crate::util::upload::BodyChunk<u32>;

struct Driver<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> {
    io: T,
    reader: FrameReader,
    writer: FrameWriter,
    encoder: hpack::Encoder,
    decoder: hpack::Decoder,
    peer_settings: PeerSettings,
    peer_greeted: bool,
    local_settings_acked: bool,
    peer_snapshot: Arc<PeerSettingsSnapshot>,
    conn_send_window: i64,
    conn_recv_window: i64,
    streams: stream_map::StreamMap,
    next_stream_id: u32,
    buffered_pending: VecDeque<u32>,
    rst_flood: RstFloodDetector,
    settings_flood: RstFloodDetector,
    control_flood: RstFloodDetector,
    config: H2Config,
    command_rx: protocol::CommandRx,
    ping_rx: mpsc::Receiver<Ping>,
    ping_armed: bool,
    closed: Arc<AtomicBool>,
    unanswered_timeout: Arc<AtomicBool>,
    load: Arc<crate::h2::client::handle::Streams>,
    peer_goaway_last_stream: Option<u32>,
    pending: VecDeque<DriverCommand>,
    shutdown_started: bool,
    body_chunks: Option<(mpsc::Sender<BodyChunkIn>, mpsc::Receiver<BodyChunkIn>)>,
    stalled: usize,
    output: output::OutputState,
    ping_seq: u64,
    pings: VecDeque<([u8; 8], oneshot::Sender<()>, std::time::Instant)>,
}

fn send_err_to_sink(sink: ResponseSink, err: H2Error) {
    match sink {
        ResponseSink::Buffered(tx) | ResponseSink::Adaptive { tx, .. } => {
            drop(tx.send(Err(err)));
        }
        ResponseSink::StreamingEx { headers_tx, .. } => {
            if let Some(tx) = headers_tx {
                drop(tx.send(Err(err)));
            }
        }
    }
}

pub(super) fn map_state_err(stream_id: u32, e: StreamStateError) -> H2Error {
    let StreamStateError::InvalidTransition { from, event } = e;
    tracing::debug!(
        stream_id,
        from,
        event,
        "stream state machine rejected transition"
    );
    H2Error::Stream {
        stream_id,
        code: ErrorCode::ProtocolError,
    }
}
