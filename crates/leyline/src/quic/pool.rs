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
    #[allow(clippy::too_many_arguments)]
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
pub struct H3DriverTask(#[allow(dead_code)] tokio::task::JoinHandle<()>);

/// Per-request-stream bookkeeping owned by the driver.
struct H3Stream {
    resp_tx: Option<oneshot::Sender<Result<H3Response, String>>>,
    status: u16,
    headers: Vec<(String, String)>,
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
            status: 0,
            headers: Vec::new(),
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
}

/// Establish a fresh pooled HTTP/3 connection to `(host, port)` and spawn its
/// driver. Drives the QUIC + H3 handshake to *established* before returning,
/// so the caller knows the connection is usable (the property a true
/// connection-level race relies on) and can send the first request straight
/// away.
pub(crate) async fn open_fresh_h3(
    h3_cfg: &H3Config,
    profile: &BrowserProfile,
    host: &str,
    port: u16,
) -> Result<(H3Client, H3DriverTask, TlsInfo), String> {
    let established = connect_and_handshake(h3_cfg, profile, host, port).await?;
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

impl H3Driver {
    async fn run(self) {
        let EstablishedH3 {
            socket,
            mut conn,
            mut h3,
            peer_addr,
            local_addr,
            max_udp_payload,
            max_response_body_bytes,
            tls: _,
        } = self.established;
        let mut command_rx = self.command_rx;
        let body_chunk_tx = self.body_chunk_tx;
        let mut body_chunk_rx = self.body_chunk_rx;
        let closed = self.closed;
        let mut streams = self.streams;

        let mut out = vec![0u8; max_udp_payload];
        let mut buf = vec![0u8; 65_535];
        let mut pending: VecDeque<H3Command> = VecDeque::new();
        let mut commands_closed = false;

        loop {
            // Reap any stream whose caller dropped its receiver (an outer timeout
            // fired, or the request was cancelled) before doing per-stream work,
            // so a freed QUIC stream-credit slot is available to a request started
            // in this same iteration.
            sweep_cancelled_streams(&mut conn, &mut streams);

            // Start any queued requests now that the connection can take them,
            // (re)attempt flow-control-parked request bodies, then push any
            // ready streaming-response body into its consumer channel.
            start_pending(
                &mut h3,
                &mut conn,
                &mut streams,
                &mut pending,
                &body_chunk_tx,
            );
            write_pending_request_bodies(&mut h3, &mut conn, &mut streams);
            let stream_backpressured = pump_streaming_bodies(
                &mut h3,
                &mut conn,
                &mut streams,
                &mut buf,
                max_response_body_bytes,
            );

            if let Err(e) = flush_egress(&socket, &mut conn, &mut out).await {
                fail_all(&mut streams, &mut pending, &closed, e);
                return;
            }

            if conn.is_closed() {
                let reason = close_reason("h3", 0, &conn);
                fail_all(&mut streams, &mut pending, &closed, reason);
                return;
            }

            // Graceful close: all handles dropped, nothing left in flight.
            if commands_closed && streams.is_empty() && pending.is_empty() {
                let _ = conn.close(true, 0x100, b"done");
                let _ = flush_egress(&socket, &mut conn, &mut out).await;
                closed.store(true, Ordering::Release);
                return;
            }

            let mut timeout = conn.timeout().unwrap_or(Duration::from_secs(5));
            // While a streaming response is back-pressured, nothing wakes the
            // driver when the consumer drains the full channel — cap the wait so
            // the pump retries promptly instead of stalling to the idle timeout.
            if stream_backpressured {
                timeout = timeout.min(STREAM_PUMP_INTERVAL);
            }
            // While any stream is in flight, cap the wait so a caller that drops
            // its receiver on an otherwise-idle connection is reaped by the next
            // `sweep_cancelled_streams` within a bounded window, rather than
            // holding stream credit until the QUIC idle timeout.
            if !streams.is_empty() {
                timeout = timeout.min(CANCEL_SWEEP_INTERVAL);
            }

            tokio::select! {
                cmd = command_rx.recv(), if !commands_closed => match cmd {
                    Some(cmd) => pending.push_back(cmd),
                    None => commands_closed = true,
                },
                chunk = body_chunk_rx.recv() => {
                    // The driver holds `body_chunk_tx`, so `recv` never yields
                    // `None`; a missing chunk is impossible here.
                    if let Some(chunk) = chunk {
                        on_request_body_chunk(&mut conn, &mut streams, chunk);
                    }
                }
                recv = socket.recv(&mut buf) => match recv {
                    Ok(len) => {
                        let recv_info = quiche::RecvInfo { from: peer_addr, to: local_addr };
                        if let Err(e) = conn.recv(&mut buf[..len], recv_info) {
                            fail_all(&mut streams, &mut pending, &closed, format!("quic recv: {e}"));
                            return;
                        }
                        if let Err(e) = drain_h3_events(
                            &mut h3,
                            &mut conn,
                            &mut streams,
                            &mut buf,
                            max_response_body_bytes,
                        ) {
                            fail_all(&mut streams, &mut pending, &closed, e);
                            return;
                        }
                    }
                    Err(e) => {
                        fail_all(&mut streams, &mut pending, &closed, format!("udp recv: {e}"));
                        return;
                    }
                },
                _ = tokio::time::sleep(timeout) => conn.on_timeout(),
            }
        }
    }
}

/// Open request streams for queued commands. Stops (leaving the rest queued)
/// the moment the connection won't accept another stream, and retries on the
/// next loop iteration once a MAX_STREAMS update arrives.
fn start_pending(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    streams: &mut HashMap<u64, H3Stream>,
    pending: &mut VecDeque<H3Command>,
    body_chunk_tx: &mpsc::Sender<H3BodyChunk>,
) {
    while let Some(H3Command::Request {
        headers,
        body,
        body_stream,
        ..
    }) = pending.front()
    {
        let streaming = body_stream.is_some();
        // FIN rides HEADERS only with no body at all; a buffered body finishes
        // on its last DATA, and a streaming body on its EOF — never here.
        let fin = !streaming && body.as_ref().is_none_or(Bytes::is_empty);
        match h3.send_request(conn, headers, fin) {
            Ok(stream_id) => {
                let Some(H3Command::Request {
                    body,
                    body_stream,
                    stream_body_tx,
                    resp_tx,
                    ..
                }) = pending.pop_front()
                else {
                    unreachable!("front matched Request above");
                };
                let mut stream = H3Stream::new(resp_tx, body, stream_body_tx, streaming);
                // A streaming body's chunks arrive on a pump task that tags them
                // with this now-known stream id and relays them to the driver.
                // The driver keeps the pump's abort handle (so teardown can
                // cancel it) and its byte-credit (so it can grant back-pressure).
                if let Some(body_stream) = body_stream {
                    let credit = Arc::new(Semaphore::new(UPLOAD_WINDOW));
                    let pump = tokio::spawn(pump_request_body(
                        stream_id,
                        body_stream,
                        body_chunk_tx.clone(),
                        Arc::clone(&credit),
                    ));
                    stream.upload_credit = Some(credit);
                    stream.pump = Some(pump.abort_handle());
                }
                write_request_body(h3, conn, stream_id, &mut stream);
                streams.insert(stream_id, stream);
            }
            // Stream limit reached — retry after the next MAX_STREAMS update.
            Err(quiche::h3::Error::StreamBlocked) | Err(quiche::h3::Error::Done) => break,
            Err(e) => {
                if let Some(H3Command::Request { resp_tx, .. }) = pending.pop_front() {
                    let _ = resp_tx.send(Err(format!("h3 send_request: {e}")));
                }
            }
        }
    }
}

/// Write any queued request-body bytes — chunks that flow control parked
/// mid-write, plus chunks freshly relayed from a streaming body's pump.
fn write_pending_request_bodies(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    streams: &mut HashMap<u64, H3Stream>,
) {
    for (stream_id, stream) in streams.iter_mut() {
        if stream.body_write_pending() {
            write_request_body(h3, conn, *stream_id, stream);
        }
    }
}

/// Write as much of a stream's queued request body as flow control allows.
///
/// The terminating FIN rides the final bytes only once the body is complete
/// (`body_eof`) and this is the last queued chunk — so a streaming body never
/// finishes early on an interior chunk. quiche applies the FIN only when the
/// whole buffer is flushed, so a partial write simply retries next loop. If the
/// body completed but the queue is already empty (an empty streaming body, or a
/// final chunk written before EOF was known), an explicit empty-FIN write
/// closes the send side.
fn write_request_body(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    stream_id: u64,
    stream: &mut H3Stream,
) {
    while let Some(front) = stream.out_chunks.front() {
        let remaining = &front[stream.out_offset..];
        let last_chunk = stream.body_eof && stream.out_chunks.len() == 1;
        match h3.send_body(conn, stream_id, remaining, last_chunk) {
            Ok(0) => return, // flow control parked; retry next loop
            Ok(written) => {
                stream.out_offset += written;
                // Return byte-credit for bytes now on the wire so the pump may
                // relay more (streaming bodies only; buffered have no credit).
                if let Some(credit) = &stream.upload_credit {
                    credit.add_permits(written);
                }
                if stream.out_offset >= front.len() {
                    stream.out_chunks.pop_front();
                    stream.out_offset = 0;
                    if last_chunk {
                        stream.fin_sent = true;
                    }
                }
            }
            Err(quiche::h3::Error::Done) | Err(quiche::h3::Error::StreamBlocked) => return,
            Err(e) => {
                stream.deliver(Err(format!("h3 send_body: {e}")));
                // Leave the dead stream in the map; the peer Reset / connection
                // teardown removes it. Stop trying to write it.
                stream.out_chunks.clear();
                stream.out_offset = 0;
                stream.fin_sent = true;
                return;
            }
        }
    }

    // Queue drained but the completed body's FIN hasn't gone out yet.
    if stream.body_eof && !stream.fin_sent {
        match h3.send_body(conn, stream_id, &[], true) {
            Ok(_) => stream.fin_sent = true,
            Err(quiche::h3::Error::Done) | Err(quiche::h3::Error::StreamBlocked) => {}
            Err(e) => {
                stream.deliver(Err(format!("h3 send_body fin: {e}")));
                stream.fin_sent = true;
            }
        }
    }
}

/// Read a streaming request body and relay each chunk to the driver tagged with
/// `stream_id`. Runs on its own task so the driver's single-owner invariant
/// holds — the body `Stream`'s `.await` never blocks the connection loop.
async fn pump_request_body(
    stream_id: u64,
    mut body: H3RequestBodyStream,
    tx: mpsc::Sender<H3BodyChunk>,
    credit: Arc<Semaphore>,
) {
    use futures_util::StreamExt;
    while let Some(item) = body.next().await {
        match item {
            Ok(mut data) => {
                // Relay in <= UPLOAD_CHUNK slices, acquiring byte-credit before
                // each. Credit caps in-flight upload bytes at UPLOAD_WINDOW; the
                // driver returns it as bytes reach the wire, so a flow-control-
                // stalled peer blocks this acquire and back-pressures the source.
                while !data.is_empty() {
                    let take = data.len().min(UPLOAD_CHUNK);
                    let slice = data.split_to(take);
                    let Ok(permit) = credit.acquire_many(take as u32).await else {
                        return; // stream torn down
                    };
                    permit.forget(); // returned by the driver via add_permits
                    if tx
                        .send(H3BodyChunk::Chunk {
                            stream_id,
                            data: slice,
                        })
                        .await
                        .is_err()
                    {
                        return; // driver gone
                    }
                }
            }
            Err(error) => {
                let _ = tx
                    .send(H3BodyChunk::Eof {
                        stream_id,
                        error: Some(error),
                    })
                    .await;
                return;
            }
        }
    }
    let _ = tx
        .send(H3BodyChunk::Eof {
            stream_id,
            error: None,
        })
        .await;
}

/// Apply a relayed request-body chunk to its stream. Appends bytes (the loop
/// flushes them via [`write_pending_request_bodies`]), or on EOF either marks
/// the body complete (FIN now allowed) or, on a source error, resets the send
/// side and fails the request.
fn on_request_body_chunk(
    conn: &mut quiche::Connection,
    streams: &mut HashMap<u64, H3Stream>,
    chunk: H3BodyChunk,
) {
    match chunk {
        H3BodyChunk::Chunk { stream_id, data } => {
            // A chunk for a stream that's already gone (reset/finished) is
            // dropped; its pump self-terminates when the body ends.
            if let Some(stream) = streams.get_mut(&stream_id) {
                stream.out_chunks.push_back(data);
            }
        }
        H3BodyChunk::Eof { stream_id, error } => {
            let Some(stream) = streams.get_mut(&stream_id) else {
                return;
            };
            match error {
                None => stream.body_eof = true,
                Some(e) => {
                    // The body source failed mid-upload (the pump self-terminated
                    // by sending this error Eof). Reset both halves — RESET_STREAM
                    // our send side, STOP_SENDING the response we'll never read —
                    // and fail the request.
                    let _ = conn.stream_shutdown(stream_id, quiche::Shutdown::Write, 0);
                    let _ = conn.stream_shutdown(stream_id, quiche::Shutdown::Read, 0);
                    let msg = format!("h3 request body stream error: {e}");
                    if stream.head_sent {
                        if let Some(tx) = &stream.stream_tx {
                            deliver_stream_error(tx, std::io::Error::other(msg));
                        }
                    } else {
                        stream.deliver(Err(msg));
                    }
                    streams.remove(&stream_id);
                }
            }
        }
    }
}

/// Reset the request-upload (write) half of a stream and cancel its pump. Used
/// when a stream is torn down for a read-side reason — the peer responded and
/// finished early, or the response consumer dropped its receiver — while an
/// upload is still in flight, so the peer sees a RESET_STREAM rather than a
/// silently abandoned half-open send side, and the pump stops at once.
fn reset_upload_half(conn: &mut quiche::Connection, stream_id: u64, stream: &mut H3Stream) {
    if stream.send_side_open() {
        let _ = conn.stream_shutdown(stream_id, quiche::Shutdown::Write, 0);
    }
    stream.cancel_upload();
}

/// True when the caller has abandoned this stream: it dropped the response
/// oneshot before the head was delivered (a buffered request, or a streaming one
/// pre-head), or — once the head has been streamed — dropped the body-channel
/// receiver. Either is the signal that an outer timeout (`response_header` /
/// `total`) or an explicit cancellation fired and the stream should be torn down.
fn stream_is_cancelled(stream: &H3Stream) -> bool {
    match stream.resp_tx.as_ref() {
        Some(tx) => tx.is_closed(),
        None => stream
            .stream_tx
            .as_ref()
            .map(|tx| tx.is_closed())
            .unwrap_or(false),
    }
}

/// Ids of streams whose caller has dropped its receiver. Split out from
/// [`sweep_cancelled_streams`] so the selection logic is unit-testable without a
/// live `quiche::Connection`.
fn cancelled_stream_ids(streams: &HashMap<u64, H3Stream>) -> Vec<u64> {
    streams
        .iter()
        .filter(|(_, s)| stream_is_cancelled(s))
        .map(|(&id, _)| id)
        .collect()
}

/// Reap streams whose caller dropped its receiver, freeing the QUIC stream-credit
/// slot instead of letting an orphan linger until the connection's idle timeout.
///
/// Without this, an outer timeout firing before the peer replies — the
/// silent-proxy case `response_header` exists to catch — leaves the stream in the
/// map with no peer event to remove it, holding `max_concurrent_bidi_streams`
/// credit and flow-control window. STOP_SENDING (`Shutdown::Read`) abandons the
/// response we will never read; [`reset_upload_half`] RESET_STREAMs the send half
/// if the upload is still open and stops the body pump. The H2 driver's
/// `sweep_cancelled_streams` is the counterpart this mirrors.
fn sweep_cancelled_streams(conn: &mut quiche::Connection, streams: &mut HashMap<u64, H3Stream>) {
    for id in cancelled_stream_ids(streams) {
        if let Some(mut stream) = streams.remove(&id) {
            let _ = conn.stream_shutdown(id, quiche::Shutdown::Read, 0);
            reset_upload_half(conn, id, &mut stream);
        }
    }
}

/// Drain all ready HTTP/3 events, dispatching each to its request stream.
/// Returns `Err` only on a connection-fatal HTTP/3 error.
fn drain_h3_events(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    streams: &mut HashMap<u64, H3Stream>,
    scratch: &mut [u8],
    max_response_body_bytes: u64,
) -> Result<(), String> {
    loop {
        match h3.poll(conn) {
            Ok((stream_id, quiche::h3::Event::Headers { list, .. })) => {
                let Some(stream) = streams.get_mut(&stream_id) else {
                    continue;
                };
                for h in &list {
                    let name = String::from_utf8_lossy(h.name()).to_string();
                    let value = String::from_utf8_lossy(h.value()).to_string();
                    if name == ":status" {
                        stream.status = value.parse().unwrap_or(0);
                    } else {
                        stream.headers.push((name, value));
                    }
                }
                // Streaming: hand the caller the head as soon as it arrives;
                // body chunks then flow through the channel via the pump.
                if stream.is_streaming() && !stream.head_sent {
                    stream.deliver_head();
                }
            }
            Ok((stream_id, quiche::h3::Event::Data)) => {
                let Some(stream) = streams.get_mut(&stream_id) else {
                    // Drain quiche's buffer for an unknown stream so it does
                    // not wedge, but discard the bytes.
                    while let Ok(n) = h3.recv_body(conn, stream_id, scratch) {
                        if n == 0 {
                            break;
                        }
                    }
                    continue;
                };
                // Streaming: drain inline, matching the buffered path's
                // `recv_body` timing so flow-control credit is granted in
                // immediate response to this packet. `forward_stream_body`
                // applies channel back-pressure (stashing one chunk and leaving
                // the rest in quiche so QUIC flow control throttles the origin).
                if stream.is_streaming() {
                    if forward_stream_body(
                        h3,
                        conn,
                        stream_id,
                        stream,
                        scratch,
                        max_response_body_bytes,
                    ) {
                        streams.remove(&stream_id);
                    }
                    continue;
                }
                while let Ok(n) = h3.recv_body(conn, stream_id, scratch) {
                    if n == 0 {
                        break;
                    }
                    // Bound the buffered body — an unbounded QUIC flow-control
                    // window otherwise lets a malicious origin OOM the client.
                    // Note: per-stream cap; aggregate across multiplexed
                    // streams is bounded by caller concurrency (the origin
                    // can't open client-initiated request streams), matching the
                    // H2 path. Add a connection-wide budget if a single host's
                    // concurrent responses need a tighter ceiling.
                    if let Err(new_len) =
                        check_body_budget(stream.body_bytes_seen, n, max_response_body_bytes)
                    {
                        let _ = conn.stream_shutdown(
                            stream_id,
                            quiche::Shutdown::Read,
                            quiche::h3::WireErrorCode::ExcessiveLoad as u64,
                        );
                        stream.deliver(Err(format!(
                            "h3: response body exceeded max_response_body_bytes ({new_len} > {max_response_body_bytes})"
                        )));
                        streams.remove(&stream_id);
                        break;
                    }
                    stream.body_bytes_seen += n;
                    stream.body.extend_from_slice(&scratch[..n]);
                }
            }
            Ok((stream_id, quiche::h3::Event::Finished)) => {
                let streaming = streams.get(&stream_id).map(H3Stream::is_streaming);
                match streaming {
                    // Streaming: mark finished; the pump drains the remaining
                    // body, then closes the channel (EOF) and removes the stream.
                    Some(true) => {
                        if let Some(stream) = streams.get_mut(&stream_id) {
                            // Peer responded before we finished uploading our
                            // request body (an early 4xx/413) — RESET our send
                            // half and cancel the pump so it stops producing into
                            // a stream we'll never finish, then keep draining the
                            // response body that's still arriving.
                            reset_upload_half(conn, stream_id, stream);
                            stream.peer_finished = true;
                        }
                    }
                    // Buffered: deliver the whole response now.
                    Some(false) => {
                        if let Some(mut stream) = streams.remove(&stream_id) {
                            // Peer responded before we finished uploading (an
                            // early 4xx/413) — abort our send side so the
                            // half-open stream doesn't leak QUIC stream credit.
                            if stream.send_side_open() {
                                let _ = conn.stream_shutdown(stream_id, quiche::Shutdown::Write, 0);
                            }
                            let resp = H3Response {
                                status: stream.status,
                                headers: std::mem::take(&mut stream.headers),
                                body: std::mem::take(&mut stream.body),
                            };
                            stream.deliver(Ok(resp));
                        }
                    }
                    None => {}
                }
            }
            Ok((stream_id, quiche::h3::Event::Reset(e))) => {
                if let Some(mut stream) = streams.remove(&stream_id) {
                    let msg = format!("h3 stream reset: {e}");
                    if stream.head_sent {
                        // Streaming head already delivered — surface the error
                        // through the body channel as a final item (reliable
                        // delivery, so a full channel doesn't drop it into a
                        // silent EOF).
                        if let Some(tx) = &stream.stream_tx {
                            deliver_stream_error(tx, std::io::Error::other(msg));
                        }
                    } else {
                        stream.deliver(Err(msg));
                    }
                }
            }
            Ok((_, quiche::h3::Event::GoAway)) | Ok((_, quiche::h3::Event::PriorityUpdate)) => {}
            Err(quiche::h3::Error::Done) => return Ok(()),
            Err(e) => return Err(format!("h3 poll: {e}")),
        }
    }
}

/// Drain ready body bytes for one streaming response into its bounded channel,
/// applying back-pressure: a chunk the channel can't accept yet is stashed
/// (`stalled`) and reading stops immediately, leaving the rest in quiche so
/// QUIC flow control throttles the origin. Returns `true` when the stream is
/// finished and should be removed (EOF channel-close, body-cap hit, or the
/// consumer dropped its receiver).
///
/// Called inline from the `Data` event — matching the buffered path's
/// `recv_body` timing so flow-control credit is granted in immediate response
/// to each packet — and again from `pump_streaming_bodies` to retry a stalled
/// chunk and emit EOF once the peer has finished.
fn forward_stream_body(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    stream_id: u64,
    stream: &mut H3Stream,
    scratch: &mut [u8],
    max_response_body_bytes: u64,
) -> bool {
    use tokio::sync::mpsc::error::TrySendError;

    let Some(tx) = stream.stream_tx.clone() else {
        return false; // buffered stream
    };

    // 1. Retry a back-pressure-stalled chunk before reading more.
    if let Some(chunk) = stream.stalled.take() {
        match tx.try_send(Ok(chunk)) {
            Ok(()) => {}
            Err(TrySendError::Full(item)) => {
                if let Ok(b) = item {
                    stream.stalled = Some(b);
                }
                return false; // still full; try again next loop
            }
            Err(TrySendError::Closed(_)) => {
                // Response consumer dropped its receiver — STOP_SENDING the
                // response and RESET any still-active upload, then remove.
                let _ = conn.stream_shutdown(stream_id, quiche::Shutdown::Read, 0);
                reset_upload_half(conn, stream_id, stream);
                return true;
            }
        }
    }

    // 2. Drain quiche into the channel until it's full or empty.
    let mut drained_clean = false;
    loop {
        match h3.recv_body(conn, stream_id, scratch) {
            Ok(0) => {
                drained_clean = true;
                break;
            }
            Ok(n) => {
                if let Err(new_len) =
                    check_body_budget(stream.body_bytes_seen, n, max_response_body_bytes)
                {
                    let _ = conn.stream_shutdown(
                        stream_id,
                        quiche::Shutdown::Read,
                        quiche::h3::WireErrorCode::ExcessiveLoad as u64,
                    );
                    deliver_stream_error(
                        &tx,
                        std::io::Error::other(format!(
                            "h3: response body exceeded max_response_body_bytes ({new_len} > {max_response_body_bytes})"
                        )),
                    );
                    return true;
                }
                stream.body_bytes_seen += n;
                let chunk = Bytes::copy_from_slice(&scratch[..n]);
                match tx.try_send(Ok(chunk)) {
                    Ok(()) => {}
                    Err(TrySendError::Full(item)) => {
                        if let Ok(b) = item {
                            stream.stalled = Some(b);
                        }
                        break; // back-pressure: stop draining this stream
                    }
                    Err(TrySendError::Closed(_)) => {
                        // Response consumer dropped its receiver — STOP_SENDING
                        // the response and RESET any still-active upload.
                        let _ = conn.stream_shutdown(stream_id, quiche::Shutdown::Read, 0);
                        reset_upload_half(conn, stream_id, stream);
                        return true;
                    }
                }
            }
            Err(quiche::h3::Error::Done) => {
                drained_clean = true;
                break;
            }
            Err(e) => {
                deliver_stream_error(&tx, std::io::Error::other(format!("h3 recv_body: {e}")));
                return true;
            }
        }
    }

    // 3. Fully drained and the peer finished → close the channel (EOF). Check
    //    the transport FIN directly, not only the H3 `Finished` event, which
    //    needs a post-drain `poll()` that never runs when the FIN rode the last
    //    packet and a back-pressured tail drained here rather than inline.
    // Note: trailers leave stream_finished false until polled → peer_finished covers them.
    if drained_clean
        && stream.stalled.is_none()
        && (stream.peer_finished || conn.stream_finished(stream_id))
    {
        stream.stream_tx = None; // drop the driver's sender → consumer EOF
        return true;
    }
    false
}

/// Retry a stalled chunk and emit EOF for every streaming response once its
/// peer has finished. The primary body read happens inline in the `Data` event
/// (see [`forward_stream_body`]); this pass exists to make progress when no new
/// packet arrives — a consumer draining a full channel, or the peer's
/// `Finished` landing after the last body was already drained.
///
/// Returns `true` if any streaming stream ended this pass back-pressured (a
/// chunk stashed because its channel is full). The driver caps its next select
/// timeout when so, since nothing else wakes it when the consumer drains a
/// full channel — without this a stalled stream waits for the idle timeout.
fn pump_streaming_bodies(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    streams: &mut HashMap<u64, H3Stream>,
    scratch: &mut [u8],
    max_response_body_bytes: u64,
) -> bool {
    let mut to_remove: Vec<u64> = Vec::new();
    for (stream_id, stream) in streams.iter_mut() {
        if stream.stream_tx.is_none() {
            continue; // buffered stream
        }
        if forward_stream_body(
            h3,
            conn,
            *stream_id,
            stream,
            scratch,
            max_response_body_bytes,
        ) {
            to_remove.push(*stream_id);
        }
    }

    for id in to_remove {
        streams.remove(&id);
    }

    streams.values().any(|s| s.stalled.is_some())
}

/// Deliver a terminal error to a streaming consumer reliably. A spawned task
/// awaits a free channel slot and appends the error after any already-queued
/// chunks, so a back-pressured consumer (full channel) sees the error instead
/// of the silent, truncating EOF that a dropped `try_send` leaves once the
/// driver drops the sender. If the consumer already dropped its receiver, the
/// send fails fast and the task exits.
fn deliver_stream_error(tx: &mpsc::Sender<std::io::Result<Bytes>>, err: std::io::Error) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let _ = tx.send(Err(err)).await;
    });
}

/// Fail every in-flight and queued request with `reason` and mark the
/// connection closed. Called on any connection-fatal path.
fn fail_all(
    streams: &mut HashMap<u64, H3Stream>,
    pending: &mut VecDeque<H3Command>,
    closed: &AtomicBool,
    reason: String,
) {
    closed.store(true, Ordering::Release);
    for (_, mut stream) in streams.drain() {
        if stream.head_sent {
            // Streaming, head already delivered — push the failure into the
            // body channel so the consumer sees an error, not a silent EOF that
            // would look like a complete (but truncated) body. Reliable delivery
            // (not try_send) so a back-pressured consumer still gets the error.
            if let Some(tx) = &stream.stream_tx {
                deliver_stream_error(tx, std::io::Error::other(reason.clone()));
            }
        } else {
            stream.deliver(Err(reason.clone()));
        }
    }
    for cmd in pending.drain(..) {
        let H3Command::Request { resp_tx, .. } = cmd;
        let _ = resp_tx.send(Err(reason.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_absent_bodies_have_nothing_to_send() {
        let (tx, _rx) = oneshot::channel();
        let s = H3Stream::new(tx, None, None, false);
        assert!(!s.body_write_pending());
        assert!(!s.send_side_open(), "no body → FIN already rode HEADERS");

        let (tx, _rx) = oneshot::channel();
        let s = H3Stream::new(tx, Some(Bytes::new()), None, false);
        assert!(
            !s.body_write_pending(),
            "an empty Bytes body must not park as pending"
        );
        assert!(!s.send_side_open());

        let (tx, _rx) = oneshot::channel();
        let s = H3Stream::new(tx, Some(Bytes::from_static(b"x")), None, false);
        assert!(s.body_write_pending());
        assert!(s.send_side_open(), "buffered body's FIN not sent yet");
    }

    #[test]
    fn streaming_body_pends_on_chunk_and_eof() {
        // A streaming request body parks nothing until a chunk arrives, keeps
        // its send side open until EOF, and — once EOF lands with an empty
        // queue — still pends so the empty terminating FIN is written.
        let (tx, _rx) = oneshot::channel();
        let mut s = H3Stream::new(tx, None, None, true);
        assert!(!s.body_write_pending(), "no chunks yet → nothing to write");
        assert!(s.send_side_open(), "streaming send side open until EOF");

        s.out_chunks.push_back(Bytes::from_static(b"chunk"));
        assert!(s.body_write_pending(), "queued chunk must pend");

        s.out_chunks.clear();
        s.body_eof = true;
        assert!(
            s.body_write_pending(),
            "EOF with an empty queue still pends an empty FIN"
        );
        assert!(s.send_side_open(), "FIN not actually sent until written");

        s.fin_sent = true;
        assert!(!s.body_write_pending());
        assert!(!s.send_side_open());
    }

    #[test]
    fn cancel_upload_drops_queue_and_finishes_send_side() {
        // Early teardown (peer responded first, or the response receiver was
        // dropped) must drop queued upload bytes and mark the send side done so
        // the writer never re-touches it or re-emits a FIN. (No pump here, so
        // the abort is a no-op; the state effects are what this pins.)
        let (tx, _rx) = oneshot::channel();
        let mut s = H3Stream::new(tx, None, None, true);
        s.out_chunks
            .push_back(Bytes::from_static(b"queued upload bytes"));
        assert!(s.body_write_pending());
        assert!(s.send_side_open());

        s.cancel_upload();
        assert!(s.out_chunks.is_empty());
        assert!(!s.body_write_pending());
        assert!(
            !s.send_side_open(),
            "send side marked finished after cancel"
        );
    }

    #[tokio::test]
    async fn deliver_is_once_only() {
        let (tx, rx) = oneshot::channel();
        let mut stream = H3Stream::new(tx, None, None, false);
        stream.status = 200;
        stream.body.extend_from_slice(b"hello");
        let resp = H3Response {
            status: stream.status,
            headers: std::mem::take(&mut stream.headers),
            body: std::mem::take(&mut stream.body),
        };
        stream.deliver(Ok(resp));
        // A second delivery is a no-op (the sender was already taken), so a
        // late Reset/teardown after a clean Finished can't double-fire or panic.
        stream.deliver(Err("late teardown".into()));

        let got = rx.await.expect("sender delivered").expect("ok response");
        assert_eq!(got.status, 200);
        assert_eq!(got.body, b"hello");
    }

    #[test]
    fn cancelled_stream_ids_selects_only_dropped_receivers() {
        // The sweep's selection logic: a stream whose caller still holds the
        // response receiver is live; one whose receiver was dropped (outer
        // timeout fired, request cancelled) is reaped.
        let mut streams = HashMap::new();
        let (tx_live, _rx_live) = oneshot::channel::<Result<H3Response, String>>();
        streams.insert(1u64, H3Stream::new(tx_live, None, None, false));
        let (tx_dead, rx_dead) = oneshot::channel::<Result<H3Response, String>>();
        streams.insert(2u64, H3Stream::new(tx_dead, None, None, false));
        drop(rx_dead);

        let cancelled = cancelled_stream_ids(&streams);
        assert_eq!(
            cancelled,
            vec![2u64],
            "only the dropped-receiver stream is selected for reaping"
        );
    }

    #[tokio::test]
    async fn cancellation_tracks_resp_then_body_receiver_across_the_head() {
        // Pre-head, cancellation is the response oneshot being dropped; once the
        // head has streamed (resp_tx taken), it tracks the body-channel receiver.
        let (tx, rx) = oneshot::channel::<Result<H3Response, String>>();
        let (body_tx, body_rx) = mpsc::channel(4);
        let mut s = H3Stream::new(tx, None, Some(body_tx), true);
        assert!(
            !stream_is_cancelled(&s),
            "live resp receiver → not cancelled"
        );
        drop(rx);
        assert!(
            stream_is_cancelled(&s),
            "dropped resp receiver pre-head → cancelled"
        );

        // Deliver the head (takes resp_tx); cancellation now follows the body
        // channel. Re-seat a live receiver first so deliver_head has a sender.
        let (tx2, _rx2) = oneshot::channel::<Result<H3Response, String>>();
        s.resp_tx = Some(tx2);
        s.deliver_head();
        assert!(
            !stream_is_cancelled(&s),
            "live body receiver post-head → not cancelled"
        );
        drop(body_rx);
        assert!(
            stream_is_cancelled(&s),
            "dropped body receiver post-head → cancelled"
        );
    }

    #[test]
    fn only_provably_unsent_requests_are_retryable() {
        // The keystone of the never-double-send guarantee: a request that may
        // have reached the origin must NOT be classified as retryable, or
        // send_request_h3_pooled would replay a non-idempotent request.
        assert!(H3SendError::NotSent("connection closed".into()).is_retryable());
        assert!(!H3SendError::Failed("stream reset".into()).is_retryable());
        assert_eq!(
            H3SendError::Failed("stream reset".into()).message(),
            "stream reset"
        );
    }

    #[test]
    fn fail_all_drains_streams_and_pending_and_marks_closed() {
        let closed = AtomicBool::new(false);
        let mut streams = HashMap::new();
        let (tx, rx_stream) = oneshot::channel();
        streams.insert(1u64, H3Stream::new(tx, None, None, false));

        let mut pending = VecDeque::new();
        let (tx2, rx_pending) = oneshot::channel();
        pending.push_back(H3Command::Request {
            headers: Vec::new(),
            body: None,
            body_stream: None,
            stream_body_tx: None,
            resp_tx: tx2,
        });

        fail_all(&mut streams, &mut pending, &closed, "boom".into());

        assert!(closed.load(Ordering::Acquire));
        assert!(streams.is_empty());
        assert!(pending.is_empty());
        assert_eq!(rx_stream.blocking_recv().unwrap().unwrap_err(), "boom");
        assert_eq!(rx_pending.blocking_recv().unwrap().unwrap_err(), "boom");
    }
}
