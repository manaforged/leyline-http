//! Persistent, poolable HTTP/3 connection (driver task + cloneable handle).
//!
//! Mirrors the H2 client's actor shape ([`crate::h2::client`]): a single
//! background task — the driver — owns the QUIC + HTTP/3 connection and the
//! UDP socket, and multiplexes request streams over it. Callers interact
//! through the cloneable [`H3Client`] handle, which fans requests in over an
//! mpsc channel and receives each buffered response on a oneshot.
//!
//! Unlike a TCP keep-alive socket, an idle QUIC connection cannot just sit in
//! the pool — it must be driven continuously to answer the peer's PINGs and
//! honour the idle timer. The driver task is what keeps a pooled H3 connection
//! alive between requests; multiplexing concurrent request streams then falls
//! out of the same event loop for free.
//!
//! Response bodies are delivered buffered (whole, on completion) or
//! incrementally streamed: the head resolves as soon as HEADERS arrive and
//! body chunks flow through a bounded channel. Chunks are drained inline as
//! each `Data` event arrives (`forward_stream_body`); a full channel stops the
//! drain, leaving bytes in quiche so QUIC flow control throttles the origin,
//! and `pump_streaming_bodies` resumes a stalled stream once the consumer
//! catches up.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bytes::Bytes;
use leyline_quiche as quiche;
use quiche::h3::NameValue;
use tokio::sync::{Semaphore, mpsc, oneshot};
use tokio::task::AbortHandle;

use crate::pool::TlsInfo;
use crate::profile::BrowserProfile;
use crate::quic::config::H3Config;
use crate::quic::connection::{
    EstablishedH3, H3Response, check_body_budget, close_reason, connect_and_handshake, flush_egress,
};
use crate::tls::TlsTrustConfig;

/// Max number of outstanding request commands the driver buffers before the
/// handle's `send` applies back-pressure. Generous; real workloads rarely
/// have more than a few thousand concurrent requests to one host.
const COMMAND_CHANNEL_CAPACITY: usize = 1024;

/// Channel depth for streamed response-body chunks. Bounded so a slow consumer
/// back-pressures the driver, which stops draining that stream from quiche and
/// lets QUIC flow control throttle the origin (matches the H2 streaming path).
const STREAM_RESP_CAPACITY: usize = 32;

/// Channel depth for the driver-wide inbound request-body relay (chunks pumped
/// from streaming request bodies, tagged by stream). This is only the wakeup
/// path into the driver — upload back-pressure is enforced per stream by
/// `UPLOAD_WINDOW` byte-credit, not by this message bound.
const STREAM_REQ_CAPACITY: usize = 64;

/// Per-stream in-flight request-body budget: the cap on streamed upload bytes
/// handed to the driver but not yet written to the wire (queued in the relay
/// channel plus `out_chunks`). The pump acquires byte-credit before relaying
/// each slice and the driver returns it as quiche accepts bytes, so a
/// flow-control-stalled peer back-pressures the body source instead of growing
/// memory without bound.
const UPLOAD_WINDOW: usize = 256 * 1024;

/// Max bytes the pump relays per chunk. Caps per-message size and keeps any
/// single slice within `UPLOAD_WINDOW`, so its credit acquire can always be
/// satisfied (a chunk larger than the window would otherwise deadlock).
const UPLOAD_CHUNK: usize = 16 * 1024;

/// Re-poll interval while a streaming response is back-pressured. Short so a
/// drained channel is refilled promptly; only active while a consumer is
/// actually behind, so it isn't a steady-state poll.
const STREAM_PUMP_INTERVAL: Duration = Duration::from_millis(2);

/// Upper bound on how long a stream whose caller dropped its receiver lingers
/// before the driver reaps it (see [`sweep_cancelled_streams`]). Caps the driver
/// select wait while any stream is in flight, so an orphan on an otherwise-idle
/// connection is reset within this window instead of holding QUIC stream credit
/// until the idle timeout. Mirrors the H2 driver's sweep cadence.
const CANCEL_SWEEP_INTERVAL: Duration = Duration::from_millis(100);

/// A streaming request body: the same boxed `Stream` shape as
/// [`crate::core::Body::Stream`]. When present, the driver pumps it into the
/// request stream incrementally instead of buffering the whole body first.
pub type H3RequestBodyStream =
    std::pin::Pin<Box<dyn futures_util::Stream<Item = std::io::Result<Bytes>> + Send + 'static>>;

/// A chunk of an outbound streaming request body, relayed from a per-request
/// pump task to the driver and tagged with the stream it belongs to.
enum H3BodyChunk {
    /// More body bytes to write to `stream_id`.
    Chunk { stream_id: u64, data: Bytes },
    /// The body source ended. `error` is `Some` if it ended by erroring (the
    /// send side is reset and the request failed) rather than completing.
    Eof {
        stream_id: u64,
        error: Option<std::io::Error>,
    },
}

/// A request fanned from an [`H3Client`] handle to the driver.
enum H3Command {
    Request {
        /// Pre-built HTTP/3 header list (pseudo-headers first), owned so it
        /// crosses the channel without borrowing the caller.
        headers: Vec<quiche::h3::Header>,
        body: Option<Bytes>,
        /// `Some` for a streaming request body: the driver spawns a pump that
        /// feeds chunks in as they arrive and finishes the stream on EOF.
        /// Mutually exclusive with a non-empty `body`.
        body_stream: Option<H3RequestBodyStream>,
        /// `Some` when the caller wants the response body delivered
        /// incrementally: the head (status + headers) resolves `resp_tx` as
        /// soon as HEADERS arrive and body chunks flow through this channel.
        /// `None` buffers the whole body, delivered on `Finished`.
        stream_body_tx: Option<mpsc::Sender<std::io::Result<Bytes>>>,
        resp_tx: oneshot::Sender<Result<H3Response, String>>,
    },
}

/// Response body shape returned by [`H3Client::send_request`].
pub enum H3RespBody {
    /// Fully buffered body (delivered on stream completion).
    Buffered(Vec<u8>),
    /// Incremental body: the caller drains chunks from the receiver.
    Streaming(mpsc::Receiver<std::io::Result<Bytes>>),
}

/// Response head + body returned by [`H3Client::send_request`].
pub struct H3ResponseParts {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: H3RespBody,
}

/// Outcome of a failed [`H3Client::send_request`], distinguishing a request
/// that provably never left the client from one that may already have reached
/// the origin. Only the former is safe to retry — replaying a request that may
/// have been processed would execute a non-idempotent request (a POST /
/// checkout) twice.
pub enum H3SendError {
    /// The request was never transmitted: the connection was already known
    /// dead, or the driver had exited before the request was even queued. Safe
    /// to retry on a fresh connection.
    NotSent(String),
    /// The request may have reached the origin before the failure (a stream
    /// reset, mid-response connection loss, or driver teardown). Surfaced
    /// as-is; never auto-retried.
    Failed(String),
}

impl H3SendError {
    pub(crate) fn message(&self) -> &str {
        match self {
            H3SendError::NotSent(m) | H3SendError::Failed(m) => m,
        }
    }

    /// Only a provably-unsent request may be replayed.
    pub(crate) fn is_retryable(&self) -> bool {
        matches!(self, H3SendError::NotSent(_))
    }
}

/// Cloneable handle to a running HTTP/3 connection.
///
/// Every clone shares the same QUIC connection; concurrent `send_request`
/// calls multiplex across independent request streams. Pooled like an
/// [`crate::h2::H2Client`]: checkout clones the handle, the pool keeps the
/// canonical clone so the connection survives between requests.
#[derive(Clone)]
pub struct H3Client {
    tx: mpsc::Sender<H3Command>,
    closed: Arc<AtomicBool>,
}

impl H3Client {
    /// Send a request over a multiplexed stream and await the response head.
    /// Concurrent calls run on independent streams. With `stream_response`,
    /// the head resolves as soon as HEADERS arrive and the body is delivered
    /// incrementally; otherwise the whole body is buffered first.
    #[expect(
        clippy::too_many_arguments,
        reason = "flat per-request wire fields across one internal call path"
    )]
    pub async fn send_request(
        &self,
        method: &str,
        authority: &str,
        path: &str,
        headers: &[(String, String)],
        body: Option<Bytes>,
        body_stream: Option<H3RequestBodyStream>,
        stream_response: bool,
    ) -> Result<H3ResponseParts, H3SendError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(H3SendError::NotSent("h3 connection closed".into()));
        }

        let uri_path = if path.is_empty() { "/" } else { path };
        let mut h3_headers: Vec<quiche::h3::Header> = Vec::with_capacity(4 + headers.len());
        h3_headers.push(quiche::h3::Header::new(b":method", method.as_bytes()));
        h3_headers.push(quiche::h3::Header::new(b":scheme", b"https"));
        h3_headers.push(quiche::h3::Header::new(b":authority", authority.as_bytes()));
        h3_headers.push(quiche::h3::Header::new(b":path", uri_path.as_bytes()));
        for (k, v) in headers {
            h3_headers.push(quiche::h3::Header::new(k.as_bytes(), v.as_bytes()));
        }

        // The caller keeps the body receiver; the driver only ever sees the
        // sender. The head response carries an empty placeholder body that we
        // replace with the receiver below.
        let (stream_body_tx, stream_body_rx) = if stream_response {
            let (tx, rx) = mpsc::channel(STREAM_RESP_CAPACITY);
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

        let (resp_tx, resp_rx) = oneshot::channel();
        // tx.send failing means the driver is gone and the command never
        // entered the queue — the request provably never went out.
        self.tx
            .send(H3Command::Request {
                headers: h3_headers,
                body,
                body_stream,
                stream_body_tx,
                resp_tx,
            })
            .await
            .map_err(|_| H3SendError::NotSent("h3 driver task has exited".into()))?;

        // Past this point the driver owns the request; any failure is
        // ambiguous (it may have hit the wire), so it is not replay-safe.
        match resp_rx.await {
            Ok(Ok(head)) => Ok(H3ResponseParts {
                status: head.status,
                headers: head.headers,
                body: match stream_body_rx {
                    Some(rx) => H3RespBody::Streaming(rx),
                    None => H3RespBody::Buffered(head.body),
                },
            }),
            Ok(Err(e)) => Err(H3SendError::Failed(e)),
            Err(_) => Err(H3SendError::Failed(
                "h3 driver dropped response sender".into(),
            )),
        }
    }

    /// `true` once the driver has shut down (connection closed, IO error, or
    /// the last handle dropped). The pool checks this before handing the
    /// connection out.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}

/// Rides along in the pool entry to keep the driver task discoverable. Dropped
/// on eviction; the `JoinHandle` detaches rather than aborting, so the driver
/// is *not* killed mid-request. The task instead self-terminates: once the
/// pool's canonical `H3Client` and every in-flight request clone are dropped,
/// `command_rx` closes and the driver runs its graceful-close branch (emitting
/// CONNECTION_CLOSE). A wedge is impossible — the connection's idle timeout
/// closes it within `max_idle_timeout`, which fails the loop out either way.
///
/// Aborting on drop would defeat both: it would kill in-flight requests on an
/// evicted-but-still-busy connection (turning them into ambiguous failures)
/// and skip the graceful close.
pub struct H3DriverTask(
    #[expect(
        dead_code,
        reason = "field held so the JoinHandle drops (and thus never aborts) with the struct; never read"
    )]
    tokio::task::JoinHandle<()>,
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum H3ResponseState {
    Initial,
    Final,
    Trailers,
}

type H3Headers = Vec<(String, String)>;

enum H3HeaderBlock {
    Informational(H3Headers),
    Final { status: u16, headers: H3Headers },
    Trailers(H3Headers),
}

impl H3ResponseState {
    fn headers(&mut self, list: &[(String, String)]) -> Result<H3HeaderBlock, &'static str> {
        match self {
            Self::Initial => {
                let (status, headers) = parse_response_head(list)?;
                if (100..200).contains(&status) {
                    Ok(H3HeaderBlock::Informational(headers))
                } else {
                    *self = Self::Final;
                    Ok(H3HeaderBlock::Final { status, headers })
                }
            }
            Self::Final => {
                if list.iter().any(|(name, _)| name.starts_with(':')) {
                    return Err("h3: trailers must not contain pseudo-headers");
                }
                *self = Self::Trailers;
                Ok(H3HeaderBlock::Trailers(list.to_vec()))
            }
            Self::Trailers => Err("h3: response contains headers after trailers"),
        }
    }

    fn data(self) -> Result<(), &'static str> {
        match self {
            Self::Final => Ok(()),
            Self::Initial => Err("h3: response DATA arrived before a final response head"),
            Self::Trailers => Err("h3: response DATA arrived after trailers"),
        }
    }

    fn finish(self) -> Result<(), &'static str> {
        match self {
            Self::Initial => Err("h3: response ended before a final response head"),
            Self::Final | Self::Trailers => Ok(()),
        }
    }
}

fn parse_response_head(list: &[(String, String)]) -> Result<(u16, H3Headers), &'static str> {
    let mut status = None;
    let mut headers = Vec::with_capacity(list.len());
    let mut regular = false;

    for (name, value) in list {
        if name.starts_with(':') {
            if regular || name != ":status" || status.is_some() {
                return Err("h3: response contains malformed pseudo-headers");
            }
            if value.len() != 3 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err("h3: response contains malformed :status pseudo-header");
            }
            let code = value
                .bytes()
                .fold(0, |status, byte| status * 10 + u16::from(byte - b'0'));
            if code == 101 {
                return Err("h3: status 101 is forbidden");
            }
            status = Some(code);
        } else {
            regular = true;
            headers.push((name.clone(), value.clone()));
        }
    }

    status
        .map(|status| (status, headers))
        .ok_or("h3: response missing :status pseudo-header")
}

/// Per-request-stream bookkeeping owned by the driver.
struct H3Stream {
    resp_tx: Option<oneshot::Sender<Result<H3Response, String>>>,
    response: H3ResponseState,
    status: u16,
    headers: Vec<(String, String)>,
    informational: Vec<Vec<(String, String)>>,
    trailers: Vec<(String, String)>,
    body: Vec<u8>,
    /// Streaming response sink; `Some` ⇒ deliver the head on HEADERS and stream
    /// body chunks through this channel instead of buffering into `body`.
    stream_tx: Option<mpsc::Sender<std::io::Result<Bytes>>>,
    /// Streaming: the head (status + headers) has been delivered on `resp_tx`.
    head_sent: bool,
    /// Streaming: a chunk read from quiche that the bounded channel could not
    /// accept yet (back-pressure). Retried before reading more body.
    stalled: Option<Bytes>,
    /// Streaming: the peer's `Finished` arrived; close the body channel once
    /// the remaining buffered body has drained into it.
    peer_finished: bool,
    /// Total response-body bytes seen, for the per-response cap — the buffered
    /// `body` Vec can't measure it in streaming mode, where chunks leave.
    body_bytes_seen: usize,
    /// Outbound request-body chunks awaiting write (one for a buffered body;
    /// many, appended as they arrive, for a streaming body). The front chunk
    /// may be partially written — `out_offset` tracks how far.
    out_chunks: VecDeque<Bytes>,
    out_offset: usize,
    /// No more request-body chunks will be appended: a buffered body is
    /// complete at construction; a streaming body becomes complete when its
    /// source signals EOF. The terminating FIN may only ride once this is set.
    body_eof: bool,
    /// The stream's send side has been finished (FIN delivered to quiche) — set
    /// when the empty/absent body finished on HEADERS, when the final body
    /// chunk flushed with FIN, or after an explicit empty-FIN write.
    fin_sent: bool,
    /// Per-stream upload byte-credit for a streaming request body (`None` for a
    /// buffered body). The pump acquires credit before relaying each slice; the
    /// driver returns it as bytes reach the wire, bounding in-flight memory.
    upload_credit: Option<Arc<Semaphore>>,
    /// Handle to this stream's request-body pump task (`None` unless streaming).
    /// Aborted on `Drop` and on early teardown so a pump never outlives its
    /// stream — the driver owns what it spawned.
    pump: Option<AbortHandle>,
    /// Request retained for one transparent retry when the server answers
    /// H3_REQUEST_REJECTED (its MAX_CONCURRENT_STREAMS budget was full at
    /// open). `(headers, buffered body)`; buffered requests only — a
    /// streaming body cannot be replayed.
    retry: Option<(Vec<quiche::h3::Header>, Option<Bytes>, u8)>,
}

impl Drop for H3Stream {
    fn drop(&mut self) {
        // Removing a stream (completion, reset, teardown, connection close) must
        // not strand its pump: the body source could be unbounded and the pump
        // would otherwise spin producing chunks the driver discards.
        if let Some(pump) = self.pump.take() {
            pump.abort();
        }
    }
}

impl H3Stream {
    /// Construct a per-request stream. `body` carries a buffered request body
    /// (consumed up front); `streaming` marks a streaming request body whose
    /// chunks arrive later via the pump. The two are mutually exclusive — a
    /// streaming request passes `body = None`.
    fn new(
        resp_tx: oneshot::Sender<Result<H3Response, String>>,
        body: Option<Bytes>,
        stream_tx: Option<mpsc::Sender<std::io::Result<Bytes>>>,
        streaming: bool,
    ) -> Self {
        let mut out_chunks = VecDeque::new();
        let (body_eof, fin_sent) = if streaming {
            // Chunks arrive later; EOF and FIN are deferred to the pump.
            (false, false)
        } else {
            match body {
                Some(b) if !b.is_empty() => {
                    out_chunks.push_back(b);
                    (true, false) // FIN rides the body's final byte
                }
                // No body: HEADERS already carried FIN, so the send side is done.
                _ => (true, true),
            }
        };
        Self {
            resp_tx: Some(resp_tx),
            response: H3ResponseState::Initial,
            status: 0,
            headers: Vec::new(),
            informational: Vec::new(),
            trailers: Vec::new(),
            body: Vec::new(),
            stream_tx,
            head_sent: false,
            stalled: None,
            peer_finished: false,
            body_bytes_seen: 0,
            out_chunks,
            out_offset: 0,
            body_eof,
            fin_sent,
            upload_credit: None,
            pump: None,
            retry: None,
        }
    }

    /// Abort the request-body pump and discard any queued upload, marking the
    /// send side finished. Used when the upload is torn down before it completes
    /// (the peer responded early, or the response receiver was dropped).
    fn cancel_upload(&mut self) {
        if let Some(pump) = self.pump.take() {
            pump.abort();
        }
        self.out_chunks.clear();
        self.out_offset = 0;
        self.fin_sent = true;
    }

    fn is_streaming(&self) -> bool {
        self.stream_tx.is_some()
    }

    /// Request-body work remains: queued chunks to write, or a known-complete
    /// body whose terminating FIN hasn't been sent yet (the empty-FIN case).
    fn body_write_pending(&self) -> bool {
        !self.out_chunks.is_empty() || (self.body_eof && !self.fin_sent)
    }

    /// The stream's send side is still open — we haven't finished uploading the
    /// request body. Used to abort the send half when the peer responds early.
    fn send_side_open(&self) -> bool {
        !self.fin_sent
    }

    fn headers(&mut self, list: &[(String, String)]) -> Result<(), &'static str> {
        match self.response.headers(list)? {
            H3HeaderBlock::Informational(headers) => self.informational.push(headers),
            H3HeaderBlock::Final { status, headers } => {
                self.status = status;
                self.headers = headers;
            }
            H3HeaderBlock::Trailers(headers) => self.trailers = headers,
        }
        Ok(())
    }

    fn data(&self) -> Result<(), &'static str> {
        self.response.data()
    }

    fn finish(&self) -> Result<(), &'static str> {
        self.response.finish()
    }

    /// Deliver a head/error response on the oneshot (buffered mode, or a
    /// streaming error before the head was sent). Once-only.
    fn deliver(&mut self, result: Result<H3Response, String>) {
        if let Some(tx) = self.resp_tx.take() {
            let _ = tx.send(result);
        }
    }

    /// Streaming: deliver the head (status + headers, empty placeholder body)
    /// the first time HEADERS arrive. The caller already holds the body
    /// receiver and stitches it in.
    fn deliver_head(&mut self) {
        if let Some(tx) = self.resp_tx.take() {
            let _ = tx.send(Ok(H3Response {
                status: self.status,
                headers: std::mem::take(&mut self.headers),
                body: Vec::new(),
            }));
        }
        self.head_sent = true;
    }

    fn deliver_error(&mut self, message: String) {
        if self.head_sent {
            if let Some(tx) = &self.stream_tx {
                deliver_stream_error(tx, std::io::Error::other(message));
            }
        } else {
            self.deliver(Err(message));
        }
    }
}

/// Establish a fresh pooled HTTP/3 connection to `(host, port)` and spawn its
/// driver. Drives the QUIC + H3 handshake to *established* before returning,
/// so the caller knows the connection is usable and can send the first
/// request straight away.
pub(crate) async fn open_fresh_h3(
    h3_cfg: &H3Config,
    profile: &BrowserProfile,
    trust: &TlsTrustConfig,
    host: &str,
    port: u16,
) -> Result<(H3Client, H3DriverTask, TlsInfo), String> {
    let established = connect_and_handshake(h3_cfg, profile, trust, host, port).await?;
    // Capture the handshake TLS detail before `established` moves into the
    // driver, so the pool can surface it on every response over this connection.
    let tls = established.tls.clone();

    let (tx, command_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
    let (body_chunk_tx, body_chunk_rx) = mpsc::channel(STREAM_REQ_CAPACITY);
    let closed = Arc::new(AtomicBool::new(false));

    let driver = H3Driver {
        established,
        command_rx,
        body_chunk_tx,
        body_chunk_rx,
        closed: Arc::clone(&closed),
        streams: HashMap::new(),
    };
    let task = tokio::spawn(driver.run());

    Ok((H3Client { tx, closed }, H3DriverTask(task), tls))
}

/// The driver task: sole owner of the QUIC connection, cooperative
/// multiplexing of request streams.
struct H3Driver {
    established: EstablishedH3,
    command_rx: mpsc::Receiver<H3Command>,
    /// Driver-wide relay for streaming request-body chunks. The driver keeps
    /// the sender so the receiver never closes; each pump task clones it.
    body_chunk_tx: mpsc::Sender<H3BodyChunk>,
    body_chunk_rx: mpsc::Receiver<H3BodyChunk>,
    closed: Arc<AtomicBool>,
    streams: HashMap<u64, H3Stream>,
}

mod driver;
use driver::*;

#[cfg(test)]
mod tests;
