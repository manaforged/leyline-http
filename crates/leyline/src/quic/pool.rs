//! Persistent, poolable HTTP/3 connection (driver task + cloneable handle).

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

/// Max number of outstanding request commands the driver buffers before the handle's `send` applies back-pressure.
const COMMAND_CHANNEL_CAPACITY: usize = 1024;

/// Channel depth for streamed response-body chunks.
const STREAM_RESP_CAPACITY: usize = 32;

/// Channel depth for the driver-wide inbound request-body relay (chunks pumped from streaming request bodies, tagged by stream).
const STREAM_REQ_CAPACITY: usize = 64;

/// Per-stream in-flight request-body budget: the cap on streamed upload bytes handed to the driver but not yet written to the wire (queued in the relay channel plus `out_chunks`).
const UPLOAD_WINDOW: usize = 256 * 1024;

/// Max bytes the pump relays per chunk.
const UPLOAD_CHUNK: usize = 16 * 1024;

/// Re-poll interval while a streaming response is back-pressured.
const STREAM_PUMP_INTERVAL: Duration = Duration::from_millis(2);

/// Upper bound on how long a stream whose caller dropped its receiver lingers before the driver reaps it (see [`sweep_cancelled_streams`]).
const CANCEL_SWEEP_INTERVAL: Duration = Duration::from_millis(100);

/// A streaming request body: the same boxed `Stream` shape as [`crate::core::Body::Stream`].
pub type H3RequestBodyStream =
    std::pin::Pin<Box<dyn futures_util::Stream<Item = std::io::Result<Bytes>> + Send + 'static>>;

/// A chunk of an outbound streaming request body, relayed from a per-request pump task to the driver and tagged with the stream it belongs to.
enum H3BodyChunk {
    /// More body bytes to write to `stream_id`.
    Chunk { stream_id: u64, data: Bytes },
    /// The body source ended.
    Eof {
        stream_id: u64,
        error: Option<std::io::Error>,
    },
}

/// A request fanned from an [`H3Client`] handle to the driver.
enum H3Command {
    Request {
        /// Pre-built HTTP/3 header list (pseudo-headers first), owned so it crosses the channel without borrowing the caller.
        headers: Vec<quiche::h3::Header>,
        body: Option<Bytes>,
        /// `Some` for a streaming request body: the driver spawns a pump that feeds chunks in as they arrive and finishes the stream on EOF.
        body_stream: Option<H3RequestBodyStream>,
        /// `Some` when the caller wants the response body delivered incrementally: the head (status + headers) resolves `resp_tx` as soon as HEADERS arrive and body chunks flow through this channel.
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

/// Outcome of a failed [`H3Client::send_request`], distinguishing a request that provably never left the client from one that may already have reached the origin.
pub enum H3SendError {
    /// The request was never transmitted: the connection was already known dead, or the driver had exited before the request was even queued.
    NotSent(String),
    /// The request may have reached the origin before the failure (a stream reset, mid-response connection loss, or driver teardown).
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
#[derive(Clone)]
pub struct H3Client {
    tx: mpsc::Sender<H3Command>,
    closed: Arc<AtomicBool>,
}

impl H3Client {
    /// Send a request over a multiplexed stream and await the response head.
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

        let (stream_body_tx, stream_body_rx) = if stream_response {
            let (tx, rx) = mpsc::channel(STREAM_RESP_CAPACITY);
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

        let (resp_tx, resp_rx) = oneshot::channel();
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

    /// `true` once the driver has shut down (connection closed, IO error, or the last handle dropped).
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}

/// Rides along in the pool entry to keep the driver task discoverable.
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
    /// Streaming response sink; `Some` ⇒ deliver the head on HEADERS and stream body chunks through this channel instead of buffering into `body`.
    stream_tx: Option<mpsc::Sender<std::io::Result<Bytes>>>,
    /// Streaming: the head (status + headers) has been delivered on `resp_tx`.
    head_sent: bool,
    /// Streaming: a chunk read from quiche that the bounded channel could not accept yet (back-pressure).
    stalled: Option<Bytes>,
    /// Streaming: the peer's `Finished` arrived; close the body channel once the remaining buffered body has drained into it.
    peer_finished: bool,
    /// Total response-body bytes seen, for the per-response cap — the buffered `body` Vec can't measure it in streaming mode, where chunks leave.
    body_bytes_seen: usize,
    /// Outbound request-body chunks awaiting write (one for a buffered body; many, appended as they arrive, for a streaming body).
    out_chunks: VecDeque<Bytes>,
    out_offset: usize,
    /// No more request-body chunks will be appended: a buffered body is complete at construction; a streaming body becomes complete when its source signals EOF.
    body_eof: bool,
    /// The stream's send side has been finished (FIN delivered to quiche) — set when the empty/absent body finished on HEADERS, when the final body chunk flushed with FIN, or after an explicit empty-FIN write.
    fin_sent: bool,
    /// Per-stream upload byte-credit for a streaming request body (`None` for a buffered body).
    upload_credit: Option<Arc<Semaphore>>,
    /// Handle to this stream's request-body pump task (`None` unless streaming).
    pump: Option<AbortHandle>,
    /// Request retained for one transparent retry when the server answers H3_REQUEST_REJECTED (its MAX_CONCURRENT_STREAMS budget was full at open).
    retry: Option<(Vec<quiche::h3::Header>, Option<Bytes>, u8)>,
}

impl Drop for H3Stream {
    fn drop(&mut self) {
        if let Some(pump) = self.pump.take() {
            pump.abort();
        }
    }
}

impl H3Stream {
    /// Construct a per-request stream.
    fn new(
        resp_tx: oneshot::Sender<Result<H3Response, String>>,
        body: Option<Bytes>,
        stream_tx: Option<mpsc::Sender<std::io::Result<Bytes>>>,
        streaming: bool,
    ) -> Self {
        let mut out_chunks = VecDeque::new();
        let (body_eof, fin_sent) = if streaming {
            (false, false)
        } else {
            match body {
                Some(b) if !b.is_empty() => {
                    out_chunks.push_back(b);
                    (true, false)
                }
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

    /// Abort the request-body pump and discard any queued upload, marking the send side finished.
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

    /// Request-body work remains: queued chunks to write, or a known-complete body whose terminating FIN hasn't been sent yet (the empty-FIN case).
    fn body_write_pending(&self) -> bool {
        !self.out_chunks.is_empty() || (self.body_eof && !self.fin_sent)
    }

    /// The stream's send side is still open — we haven't finished uploading the request body.
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

    /// Deliver a head/error response on the oneshot (buffered mode, or a streaming error before the head was sent).
    fn deliver(&mut self, result: Result<H3Response, String>) {
        if let Some(tx) = self.resp_tx.take() {
            let _ = tx.send(result);
        }
    }

    /// Streaming: deliver the head (status + headers, empty placeholder body) the first time HEADERS arrive.
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

/// Establish a fresh pooled HTTP/3 connection to `(host, port)` and spawn its driver.
pub(crate) async fn open_fresh_h3(
    h3_cfg: &H3Config,
    profile: &BrowserProfile,
    trust: &TlsTrustConfig,
    host: &str,
    port: u16,
) -> Result<(H3Client, H3DriverTask, TlsInfo), String> {
    let established = connect_and_handshake(h3_cfg, profile, trust, host, port).await?;
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

/// The driver task: sole owner of the QUIC connection, cooperative multiplexing of request streams.
struct H3Driver {
    established: EstablishedH3,
    command_rx: mpsc::Receiver<H3Command>,
    /// Driver-wide relay for streaming request-body chunks.
    body_chunk_tx: mpsc::Sender<H3BodyChunk>,
    body_chunk_rx: mpsc::Receiver<H3BodyChunk>,
    closed: Arc<AtomicBool>,
    streams: HashMap<u64, H3Stream>,
}

mod driver;
use driver::*;

#[cfg(test)]
mod tests;
