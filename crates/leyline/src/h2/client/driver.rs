use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};

use crate::error::BodyLimit;
use crate::h2::H2Config;
use crate::h2::H2Error;
use crate::h2::codec::{FrameReader, FrameWriter};
use crate::h2::connection::{FloodGuard, PeerSettings};
use crate::h2::error::ErrorCode;
use crate::h2::frame::*;
use crate::h2::hpack;
use crate::h2::stream_state::{StreamState, StreamStateError};
use crate::wire::ResponseMode;

use crate::wire::{ResponseState, TransportBody, TransportResponse, Upload};

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
        let (body_tx, terminal, body_rx) = crate::wire::body_channel();
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

    pub(super) fn fail(self, err: H2Error) {
        match self {
            Self::Buffered(tx)
            | Self::Adaptive { tx, .. }
            | Self::StreamingEx {
                headers_tx: Some(tx),
                ..
            } => drop(tx.send(Err(err))),
            Self::StreamingEx {
                headers_tx: None,
                terminal,
                ..
            } => drop(terminal.send(Err(io::Error::other(format!("h2 stream failed: {err}"))))),
        }
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
    head_request: bool,
    has_body: bool,
    body_limit: BodyLimit,
    pending_send: Option<PendingSend>,
    send_body_input: SendBodyInput,
    upload: Option<Upload>,
    stalled: std::collections::VecDeque<Bytes>,
    remote_done: bool,
    declared_len: Option<u64>,
    recv_len: u64,
}

impl StreamActor {
    fn new(
        send_window: i64,
        recv_window: i64,
        sink: ResponseSink,
        head_request: bool,
        body_limit: BodyLimit,
    ) -> Self {
        Self {
            state: StreamState::Idle,
            send_window,
            recv_window,
            response_tx: Some(sink),
            status: http::StatusCode::OK,
            response: ResponseState::default(),
            resp_headers: http::HeaderMap::new(),
            body: Vec::new(),
            trailers: http::HeaderMap::new(),
            head_request,
            has_body: !head_request,
            body_limit,
            pending_send: None,
            send_body_input: SendBodyInput::None,
            upload: None,
            stalled: std::collections::VecDeque::new(),
            remote_done: false,
            declared_len: None,
            recv_len: 0,
        }
    }

    fn length_mismatch(&self) -> bool {
        crate::wire::length_mismatch(self.has_body, self.declared_len, self.recv_len)
    }

    fn response(&mut self, body: Vec<u8>) -> TransportResponse {
        TransportResponse::new(
            http::Version::HTTP_2,
            self.status,
            std::mem::take(&mut self.resp_headers).into(),
            std::mem::take(&mut self.trailers),
            TransportBody::Buffered(body),
        )
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
        if let Some(sink) = self.response_tx.take() {
            sink.fail(err);
        }
    }
}

impl StreamActor {
    fn cancel_upload(&mut self) {
        self.upload = None;
        self.pending_send = None;
        self.send_body_input = SendBodyInput::None;
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
    conn_send_window: i64,
    conn_recv_window: i64,
    streams: stream_map::StreamMap,
    next_stream_id: u32,
    rst_flood: FloodGuard,
    settings_flood: FloodGuard,
    control_flood: FloodGuard,
    config: H2Config,
    command_rx: protocol::CommandRx,
    ping_rx: mpsc::Receiver<Ping>,
    ping_armed: bool,
    closed: Arc<AtomicBool>,
    unanswered_timeout: Arc<AtomicBool>,
    peer_goaway_last_stream: Option<u32>,
    pending: VecDeque<DriverCommand>,
    shutdown_started: bool,
    body_chunks: Option<(mpsc::Sender<BodyChunkIn>, mpsc::Receiver<BodyChunkIn>)>,
    output: output::OutputState,
    ping_seq: u64,
    pings: VecDeque<([u8; 8], oneshot::Sender<()>, std::time::Instant)>,
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
