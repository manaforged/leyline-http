//! HTTP/2 driver task — the concurrent core of the client.
//!
//! A single background task owns the connection's reader and writer
//! halves plus the stream table; [`super::H2Client`] handles fan commands
//! in over an mpsc channel. This module holds the driver, the per-stream
//! actor, the command protocol, the connection bootstrap (`start`), and
//! the flow-control machinery. Its internal state stays private to this
//! module — only the items the handle and connect-stream modules need are
//! widened to `pub(super)`/`pub(crate)`.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};

use crate::h2::codec::{FrameReader, FrameWriter};
use crate::h2::config::H2Config;
use crate::h2::connection::{H2Response, PeerSettings, RstFloodDetector};
use crate::h2::error::{ErrorCode, H2Error};
use crate::h2::frame::*;
use crate::h2::hpack;
use crate::h2::stream_state::{StreamState, StreamStateError};

use super::types::{H2ResponseEx, ResponseBody};

mod bootstrap;
mod command;
mod lifecycle;
mod protocol;
mod recv;
mod send;

// Re-export items from child modules for external callers.
pub(super) use self::bootstrap::pump_request_body;
pub(crate) use self::bootstrap::start;
pub use protocol::DriverTask;
pub(crate) use protocol::{checked_window_add, DriverCommand, DriverRequestBody};

/// Max number of outstanding SendRequest commands the driver will buffer
/// before applying back-pressure on callers. Generous — real workloads
/// rarely ship more than a few thousand concurrent requests to one host.
const COMMAND_CHANNEL_CAPACITY: usize = 1024;

/// Channel capacity for streaming request body chunks. Bounded so the
/// producer task applies back-pressure to the user-provided stream.
pub(super) const STREAM_REQ_BODY_CAPACITY: usize = 32;

/// Channel capacity for streaming response body chunks.
pub(super) const STREAM_RESP_BODY_CAPACITY: usize = 32;

/// Shared snapshot of peer settings visible to every cloneable handle.
///
/// The driver is the sole writer; clones only ever read. We publish the
/// fields we actually care about (window sizes, concurrency limits,
/// HPACK table size) under an atomic for lock-free reads.
#[doc(hidden)]
#[derive(Debug)]
pub struct PeerSettingsSnapshot {
    /// Peer's latest SETTINGS_MAX_CONCURRENT_STREAMS, if advertised.
    /// `0` means "none advertised" — callers must check `has_max_streams`.
    max_concurrent_streams: std::sync::atomic::AtomicU32,
    has_max_streams: std::sync::atomic::AtomicBool,
    /// RFC 8441 §3 — `SETTINGS_ENABLE_CONNECT_PROTOCOL`. Sticky-on
    /// once the driver observes a `1` from the peer. Cloneable
    /// handles read this to decide whether WebSocket-over-HTTP/2 is
    /// viable.
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

    /// Public read: has the peer advertised RFC 8441 extended CONNECT
    /// support yet?
    pub fn enable_connect_protocol(&self) -> bool {
        self.enable_connect_protocol.load(Ordering::Relaxed)
    }
}

/// A send that has partially written and is waiting for WINDOW_UPDATE
/// to resume. Lives on the [`StreamActor`] and is drained whenever the
/// connection- or stream-level send window grows.
struct PendingSend {
    /// Remaining body bytes not yet written.
    remaining: Bytes,
    /// Trailers to send after END_STREAM on the final DATA frame — or,
    /// if non-empty, a trailing HEADERS frame follows the DATA.
    trailers: Vec<(String, String)>,
}

/// Where the final response should be delivered.
enum ResponseSink {
    /// Legacy API: buffered body in a oneshot.
    Buffered(oneshot::Sender<Result<H2Response, H2Error>>),
    /// Extended API, buffered response: body collected in-driver, one
    /// final message on completion.
    BufferedEx(oneshot::Sender<Result<H2ResponseEx, H2Error>>),
    /// Extended API, streaming response: headers go out on the oneshot
    /// as soon as they arrive; body chunks flow through the mpsc.
    StreamingEx {
        headers_tx: Option<oneshot::Sender<Result<H2ResponseEx, H2Error>>>,
        body_tx: mpsc::Sender<io::Result<Bytes>>,
    },
}

/// Streaming-body input state attached to a stream actor.
enum SendBodyInput {
    /// No streaming source — entire body was provided up-front in
    /// `pending_send.remaining` or there is no body.
    None,
    /// Streaming from a producer task. The driver pushes whatever
    /// chunks arrive into `pending_buf`, which `write_body_or_park`
    /// treats as its source of bytes. `closed` is set once the
    /// producer signalled EOF (or an error was forwarded).
    Streaming {
        pending_buf: VecDeque<Bytes>,
        closed: bool,
        error: Option<io::Error>,
        /// Trailers to emit after the last chunk. Not surfaced yet
        /// through the `send_request_ex` API — reserved for a future
        /// `send_request_ex_with_trailers` entry point.
        #[allow(dead_code)]
        trailers: Vec<(String, String)>,
    },
}

/// Per-stream bookkeeping owned by the driver.
struct StreamActor {
    state: StreamState,
    send_window: i64,
    recv_window: i64,
    /// Where the driver delivers the final response.
    response_tx: Option<ResponseSink>,
    status: u16,
    got_headers: bool,
    resp_headers: Vec<(String, String)>,
    body: Vec<u8>,
    trailers: Option<Vec<(String, String)>>,
    /// HEAD/1xx/204/304: drain DATA without buffering.
    drop_body: bool,
    /// Remaining outbound body (set when flow-control parks us mid-body).
    pending_send: Option<PendingSend>,
    /// Streaming request body state, if the caller passed a stream.
    send_body_input: SendBodyInput,
    /// `true` once we've written a DATA frame with END_STREAM (or
    /// the trailing HEADERS frame).
    send_closed: bool,
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
            send_closed: false,
        }
    }

    /// Deliver the HEADERS portion of a streaming response. The caller
    /// of `send_request_ex` keeps the body receiver, so the driver
    /// sends an `H2ResponseEx` with a placeholder body; the caller
    /// stitches in the real receiver before returning from the API.
    fn deliver_headers_streaming(&mut self) {
        if let Some(ResponseSink::StreamingEx { headers_tx, .. }) = self.response_tx.as_mut() {
            if let Some(tx) = headers_tx.take() {
                let _ = tx.send(Ok(H2ResponseEx {
                    status: self.status,
                    headers: std::mem::take(&mut self.resp_headers),
                    body: ResponseBody::Buffered(Vec::new()),
                    trailers: None,
                }));
            }
        }
    }

    fn deliver_ok(&mut self) {
        match self.response_tx.take() {
            Some(ResponseSink::Buffered(tx)) => {
                let _ = tx.send(Ok(H2Response {
                    status: self.status,
                    headers: std::mem::take(&mut self.resp_headers),
                    body: std::mem::take(&mut self.body),
                    trailers: self.trailers.take(),
                }));
            }
            Some(ResponseSink::BufferedEx(tx)) => {
                let _ = tx.send(Ok(H2ResponseEx {
                    status: self.status,
                    headers: std::mem::take(&mut self.resp_headers),
                    body: ResponseBody::Buffered(std::mem::take(&mut self.body)),
                    trailers: self.trailers.take(),
                }));
            }
            Some(ResponseSink::StreamingEx { body_tx, .. }) => {
                // Streaming — closing the body channel signals EOF to the
                // caller. Trailers currently not surfaced over the
                // streaming API.
                drop(body_tx);
            }
            None => {}
        }
    }

    fn deliver_err(&mut self, err: H2Error) {
        match self.response_tx.take() {
            Some(ResponseSink::Buffered(tx)) => {
                let _ = tx.send(Err(err));
            }
            Some(ResponseSink::BufferedEx(tx)) => {
                let _ = tx.send(Err(err));
            }
            Some(ResponseSink::StreamingEx {
                headers_tx,
                body_tx,
            }) => {
                if let Some(tx) = headers_tx {
                    let _ = tx.send(Err(err));
                } else {
                    let _ =
                        body_tx.try_send(Err(io::Error::other(format!("h2 stream failed: {err}"))));
                }
            }
            None => {}
        }
    }
}

/// A streaming request body chunk routed back to the driver from a
/// per-stream producer task.
enum BodyChunkIn {
    /// A chunk of body bytes.
    Chunk { stream_id: u32, data: Bytes },
    /// Stream ended (EOF). If `error` is set the stream carried an IO
    /// error rather than a clean close.
    Eof {
        stream_id: u32,
        error: Option<io::Error>,
    },
}

/// The actor task: single writer/reader owner, cooperative multiplexing.
struct Driver<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> {
    reader: FrameReader<tokio::io::ReadHalf<T>>,
    writer: FrameWriter<tokio::io::WriteHalf<T>>,
    encoder: hpack::Encoder,
    decoder: hpack::Decoder,
    peer_settings: PeerSettings,
    peer_snapshot: Arc<PeerSettingsSnapshot>,
    conn_send_window: i64,
    conn_recv_window: i64,
    streams: HashMap<u32, StreamActor>,
    next_stream_id: u32,
    /// Stream IDs with a pending_send, in insertion order, so we can fairly
    /// resume them when flow-control credit returns.
    buffered_pending: VecDeque<u32>,
    rst_flood: RstFloodDetector,
    /// Sliding-window guard against SETTINGS floods — mid-connection
    /// non-ACK SETTINGS frames beyond the configured rate force an
    /// `ENHANCE_YOUR_CALM` close. Same detector shape as `rst_flood`.
    settings_flood: RstFloodDetector,
    config: H2Config,
    command_rx: mpsc::Receiver<DriverCommand>,
    closed: Arc<AtomicBool>,
    /// Set when the peer sends GOAWAY. New SendRequest commands are
    /// rejected; already-open streams ≤ last_stream_id may finish.
    peer_goaway_last_stream: Option<u32>,
    shutdown_started: bool,
    /// Sink given to per-stream request-body relay tasks so they can
    /// hand chunks back to the driver without needing per-stream
    /// channels in `select!`.
    body_chunk_tx: mpsc::Sender<BodyChunkIn>,
    /// The receive side the driver awaits in `event_loop`.
    body_chunk_rx: mpsc::Receiver<BodyChunkIn>,
}

fn send_err_to_sink(sink: ResponseSink, err: H2Error) {
    match sink {
        ResponseSink::Buffered(tx) => {
            let _ = tx.send(Err(err));
        }
        ResponseSink::BufferedEx(tx) => {
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

pub(super) fn clone_err(e: &H2Error) -> H2Error {
    match e {
        H2Error::Connection { code, reason } => H2Error::Connection {
            code: *code,
            reason: reason.clone(),
        },
        H2Error::Stream { stream_id, code } => H2Error::Stream {
            stream_id: *stream_id,
            code: *code,
        },
        H2Error::Io(e) => H2Error::Io(std::io::Error::new(e.kind(), e.to_string())),
        H2Error::Hpack(s) => H2Error::Hpack(s.clone()),
        H2Error::FrameTooLarge { size, max } => H2Error::FrameTooLarge {
            size: *size,
            max: *max,
        },
    }
}
