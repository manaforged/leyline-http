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
use crate::header_str::HeaderStr;

use super::types::{H2ResponseEx, ResponseBody};

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
pub(crate) use protocol::{DriverCommand, DriverRequestBody, checked_window_add};

const COMMAND_CHANNEL_CAPACITY: usize = 1024;

const PING_CHANNEL_CAPACITY: usize = 16;

pub(super) const STREAM_REQ_BODY_CAPACITY: usize = 32;

pub(super) const STREAM_RESP_BODY_CAPACITY: usize = 32;

#[doc(hidden)]
#[derive(Debug)]
pub struct PeerSettingsSnapshot {
    max_concurrent_streams: std::sync::atomic::AtomicU32,
    has_max_streams: std::sync::atomic::AtomicBool,
    enable_connect_protocol: std::sync::atomic::AtomicBool,
}

impl PeerSettingsSnapshot {
    fn new() -> Self {
        Self {
            max_concurrent_streams: std::sync::atomic::AtomicU32::new(0),
            has_max_streams: std::sync::atomic::AtomicBool::new(false),
            enable_connect_protocol: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn set_max_concurrent_streams(&self, value: Option<u32>) {
        match value {
            Some(v) => {
                self.max_concurrent_streams.store(v, Ordering::Relaxed);
                self.has_max_streams.store(true, Ordering::Relaxed);
            }
            None => self.has_max_streams.store(false, Ordering::Relaxed),
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
    Buffered(oneshot::Sender<Result<H2ResponseEx, H2Error>>),
    StreamingEx {
        headers_tx: Option<oneshot::Sender<Result<H2ResponseEx, H2Error>>>,
        body_tx: mpsc::Sender<io::Result<Bytes>>,
        terminal: mpsc::OwnedPermit<io::Result<Bytes>>,
    },
    Adaptive {
        tx: oneshot::Sender<Result<H2ResponseEx, H2Error>>,
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
        headers_tx: oneshot::Sender<Result<H2ResponseEx, H2Error>>,
    ) -> (Self, mpsc::Receiver<io::Result<Bytes>>) {
        let (sink, body_rx) = Self::adaptive(headers_tx, ResponseMode::Streamed);
        (sink.settle(0), body_rx)
    }

    pub(super) fn adaptive(
        tx: oneshot::Sender<Result<H2ResponseEx, H2Error>>,
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

    pub(super) fn settle(self, status: u16) -> Self {
        match self {
            Self::Adaptive {
                tx,
                body_tx,
                terminal,
                mode,
            } if mode.keeps_stream(status) => Self::StreamingEx {
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
    status: u16,
    got_headers: bool,
    resp_headers: Vec<(HeaderStr, HeaderStr)>,
    body: Vec<u8>,
    trailers: Option<Vec<(HeaderStr, HeaderStr)>>,
    drop_body: bool,
    pending_send: Option<PendingSend>,
    send_body_input: SendBodyInput,
    upload_credit: Option<Arc<Semaphore>>,
    pump: Option<AbortHandle>,
    send_closed: bool,
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
            status: 0,
            got_headers: false,
            resp_headers: Vec::new(),
            body: Vec::new(),
            trailers: None,
            drop_body,
            pending_send: None,
            send_body_input: SendBodyInput::None,
            upload_credit: None,
            pump: None,
            send_closed: false,
            stalled: std::collections::VecDeque::new(),
            remote_done: false,
            declared_len: None,
            recv_len: 0,
        }
    }

    fn length_mismatch(&self) -> bool {
        !self.drop_body && self.declared_len.is_some_and(|len| len != self.recv_len)
    }

    fn deliver_headers_streaming(&mut self) {
        if let Some(ResponseSink::StreamingEx { headers_tx, .. }) = self.response_tx.as_mut()
            && let Some(tx) = headers_tx.take()
        {
            let _ = tx.send(Ok(H2ResponseEx {
                status: self.status,
                headers: std::mem::take(&mut self.resp_headers),
                body: ResponseBody::Buffered(Vec::new()),
                trailers: None,
            }));
        }
    }

    fn deliver_ok(&mut self) {
        match self.response_tx.take() {
            Some(ResponseSink::Buffered(tx)) => {
                let _ = tx.send(Ok(H2ResponseEx {
                    status: self.status,
                    headers: std::mem::take(&mut self.resp_headers),
                    body: ResponseBody::Buffered(std::mem::take(&mut self.body)),
                    trailers: self.trailers.take(),
                }));
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
                let _ = tx.send(Err(err));
            }
            Some(ResponseSink::StreamingEx {
                headers_tx,
                terminal,
                ..
            }) => {
                if let Some(tx) = headers_tx {
                    let _ = tx.send(Err(err));
                } else {
                    drop(terminal.send(Err(io::Error::other(format!("h2 stream failed: {err}")))));
                }
            }
            None => {}
        }
    }
}

impl Drop for StreamActor {
    fn drop(&mut self) {
        if let Some(pump) = self.pump.take() {
            pump.abort();
        }
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
    reader: FrameReader<tokio::io::ReadHalf<T>>,
    writer: FrameWriter<tokio::io::WriteHalf<T>>,
    encoder: hpack::Encoder,
    decoder: hpack::Decoder,
    peer_settings: PeerSettings,
    peer_greeted: bool,
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
    command_rx: mpsc::Receiver<DriverCommand>,
    ping_rx: mpsc::Receiver<oneshot::Sender<()>>,
    closed: Arc<AtomicBool>,
    peer_goaway_last_stream: Option<u32>,
    pending: VecDeque<DriverCommand>,
    shutdown_started: bool,
    body_chunk_tx: mpsc::Sender<BodyChunkIn>,
    body_chunk_rx: mpsc::Receiver<BodyChunkIn>,
    stalled: usize,
    output: output::OutputState,
    ping_seq: u64,
    pings: VecDeque<([u8; 8], oneshot::Sender<()>)>,
}

fn send_err_to_sink(sink: ResponseSink, err: H2Error) {
    match sink {
        ResponseSink::Buffered(tx) | ResponseSink::Adaptive { tx, .. } => {
            let _ = tx.send(Err(err));
        }
        ResponseSink::StreamingEx { headers_tx, .. } => {
            if let Some(tx) = headers_tx {
                let _ = tx.send(Err(err));
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
