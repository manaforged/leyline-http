//! Concurrent multiplexing HTTP/2 client (driver + handle).
//!
//! A single background task — the "driver" — owns the connection's reader
//! and writer halves plus the stream table. Callers interact through the
//! cloneable [`H2Client`] handle, which fans out `send_request` calls to
//! the driver via an mpsc channel. Concurrent requests run as independent
//! streams on the same TCP connection with no head-of-line blocking.
//!
//! This module is the concurrent heart of the crate; the legacy
//! [`crate::connection::ClientConnection`] API is retained as a thin shell
//! around it for backward compatibility.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::codec::{FrameReader, FrameWriter};
use crate::config::H2Config;
use crate::connection::{
    encode_request_pseudos, id_to_u16, H2Response, PeerSettings, PseudoHeaders, RstFloodDetector,
};
use crate::error::{ErrorCode, H2Error};
use crate::frame::*;
use crate::hpack;
use crate::stream_state::{StreamEvent, StreamState, StreamStateError};

/// Max number of outstanding SendRequest commands the driver will buffer
/// before applying back-pressure on callers. Generous — real workloads
/// rarely ship more than a few thousand concurrent requests to one host.
const COMMAND_CHANNEL_CAPACITY: usize = 1024;

/// Channel capacity for streaming request body chunks. Bounded so the
/// producer task applies back-pressure to the user-provided stream.
const STREAM_REQ_BODY_CAPACITY: usize = 32;

/// Channel capacity for streaming response body chunks.
const STREAM_RESP_BODY_CAPACITY: usize = 32;

/// Request body supplied to [`H2Client::send_request_ex`].
pub enum RequestBody {
    /// No body — headers carry END_STREAM.
    None,
    /// Fully-materialised bytes. Sent as one or more DATA frames.
    Buffered(Bytes),
    /// Streaming body: chunks are pulled as they arrive from the
    /// caller-provided stream. Honours flow control and the peer's
    /// MAX_FRAME_SIZE by breaking into multiple DATA frames.
    Streaming {
        /// The stream yielding chunks.
        stream: Pin<Box<dyn futures_util::Stream<Item = io::Result<Bytes>> + Send + 'static>>,
        /// Known exact length in bytes, if any. Informational only at
        /// the H2 level — the client crate sets `content-length` before
        /// calling into h2.
        length_hint: Option<u64>,
    },
}

impl std::fmt::Debug for RequestBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => f.debug_struct("RequestBody::None").finish(),
            Self::Buffered(b) => f
                .debug_struct("RequestBody::Buffered")
                .field("len", &b.len())
                .finish(),
            Self::Streaming { length_hint, .. } => f
                .debug_struct("RequestBody::Streaming")
                .field("length_hint", length_hint)
                .finish(),
        }
    }
}

impl From<Option<Bytes>> for RequestBody {
    fn from(b: Option<Bytes>) -> Self {
        match b {
            None => RequestBody::None,
            Some(b) if b.is_empty() => RequestBody::None,
            Some(b) => RequestBody::Buffered(b),
        }
    }
}

/// Extended response returned by [`H2Client::send_request_ex`]. Carries a
/// [`ResponseBody`] that may be either buffered (the default) or a
/// streaming receiver.
#[derive(Debug)]
pub struct H2ResponseEx {
    /// HTTP status code.
    pub status: u16,
    /// Response headers in wire order.
    pub headers: Vec<(String, String)>,
    /// Response body — buffered or streaming.
    pub body: ResponseBody,
    /// Trailers, if any. Only populated for buffered responses; in the
    /// streaming path trailers are delivered as a final zero-length
    /// chunk followed by close of the channel. (Trailer delivery over
    /// the streaming API is not exposed yet — callers that need
    /// trailers should use the buffered path.)
    pub trailers: Option<Vec<(String, String)>>,
}

/// Response body shape delivered alongside an [`H2ResponseEx`].
pub enum ResponseBody {
    /// Fully buffered body (the default).
    Buffered(Vec<u8>),
    /// Streaming body: the caller drains chunks via the receiver.
    Streaming(mpsc::Receiver<io::Result<Bytes>>),
}

impl std::fmt::Debug for ResponseBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Buffered(b) => f
                .debug_struct("ResponseBody::Buffered")
                .field("len", &b.len())
                .finish(),
            Self::Streaming(_) => f
                .debug_struct("ResponseBody::Streaming")
                .finish_non_exhaustive(),
        }
    }
}

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

/// Cloneable handle to a running HTTP/2 connection.
///
/// Every clone shares the same connection. Concurrent `send_request`
/// calls on one or many clones are multiplexed across independent
/// streams on the single underlying TCP connection with no
/// head-of-line blocking between streams.
#[derive(Clone)]
pub struct H2Client {
    tx: mpsc::Sender<DriverCommand>,
    closed: Arc<AtomicBool>,
    peer_settings: Arc<PeerSettingsSnapshot>,
}

impl H2Client {
    /// Send a request over a multiplexed stream and await the response.
    ///
    /// Concurrent calls run in parallel on independent streams; one
    /// stream's flow-control stall does not block others.
    pub async fn send_request(
        &self,
        pseudo: PseudoHeaders,
        headers: Vec<(String, String)>,
        body: Option<Bytes>,
    ) -> Result<H2Response, H2Error> {
        self.send_request_with_trailers(pseudo, headers, body, Vec::new())
            .await
    }

    /// Send a request with optional trailers and await the response.
    ///
    /// Empty `trailers` is equivalent to [`Self::send_request`]. Non-empty
    /// trailers emit a terminating HEADERS frame with END_STREAM after
    /// the request body per RFC 9113 §8.1.
    pub async fn send_request_with_trailers(
        &self,
        pseudo: PseudoHeaders,
        headers: Vec<(String, String)>,
        body: Option<Bytes>,
        trailers: Vec<(String, String)>,
    ) -> Result<H2Response, H2Error> {
        if self.closed.load(Ordering::Acquire) {
            return Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "connection closed".into(),
            });
        }

        let (response_tx, response_rx) = oneshot::channel();
        let cmd = DriverCommand::SendRequest {
            pseudo,
            headers,
            body,
            trailers,
            response_tx,
        };
        self.tx
            .send(cmd)
            .await
            .map_err(|_| H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "driver task has exited".into(),
            })?;

        match response_rx.await {
            Ok(result) => result,
            Err(_) => Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "driver dropped response sender".into(),
            }),
        }
    }

    /// Extended send — supports streaming request bodies and optional
    /// streaming response delivery.
    ///
    /// When `body` is [`RequestBody::Streaming`], chunks are pumped to
    /// the driver via an mpsc channel; the driver honours flow-control
    /// as usual, parking the stream when the send window is empty.
    ///
    /// When `stream_response` is `true`, the oneshot resolves as soon as
    /// HEADERS arrive; body chunks are delivered via
    /// [`ResponseBody::Streaming`]. The caller is responsible for
    /// draining the receiver — the driver applies back-pressure through
    /// the bounded channel.
    pub async fn send_request_ex(
        &self,
        pseudo: PseudoHeaders,
        headers: Vec<(String, String)>,
        body: RequestBody,
        stream_response: bool,
    ) -> Result<H2ResponseEx, H2Error> {
        if self.closed.load(Ordering::Acquire) {
            return Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "connection closed".into(),
            });
        }

        // Convert the caller-supplied stream into an mpsc receiver that
        // the driver can pull from. A small producer task owns the
        // `Stream` object — the driver never awaits on user code.
        let body_in = match body {
            RequestBody::None => DriverRequestBody::None,
            RequestBody::Buffered(b) => DriverRequestBody::Buffered(b),
            RequestBody::Streaming {
                stream,
                length_hint,
            } => {
                let (body_tx, body_rx) = mpsc::channel(STREAM_REQ_BODY_CAPACITY);
                tokio::spawn(pump_request_body(stream, body_tx));
                DriverRequestBody::Streaming {
                    rx: body_rx,
                    length_hint,
                }
            }
        };

        // Streaming-response channel is created here so the caller keeps
        // the receiver while the driver only sees the sender. When the
        // driver resolves the oneshot, we stitch the receiver into the
        // returned `H2ResponseEx`.
        let (response_tx, response_rx) = oneshot::channel::<Result<H2ResponseEx, H2Error>>();
        let (body_body_tx, body_body_rx) = mpsc::channel::<io::Result<Bytes>>(
            STREAM_RESP_BODY_CAPACITY,
        );

        let cmd = DriverCommand::SendRequestEx {
            pseudo,
            headers,
            body: body_in,
            stream_response,
            response_tx,
            stream_body_tx: body_body_tx,
        };
        self.tx
            .send(cmd)
            .await
            .map_err(|_| H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "driver task has exited".into(),
            })?;

        match response_rx.await {
            Ok(Ok(mut resp)) => {
                if stream_response {
                    // Replace the body payload with the receiver the
                    // caller holds. The driver's ResponseBody on the
                    // oneshot is a placeholder.
                    resp.body = ResponseBody::Streaming(body_body_rx);
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

    /// Return `true` if the driver task has shut down (GOAWAY, IO error,
    /// or last handle dropped).
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// Return `true` if the peer has advertised
    /// `SETTINGS_ENABLE_CONNECT_PROTOCOL = 1` (RFC 8441 §3). Callers
    /// consult this before attempting an extended-CONNECT open;
    /// `false` (the default) means the server speaks only classic
    /// CONNECT and WebSocket clients should fall back to a fresh
    /// HTTP/1.1 TLS connection.
    pub fn peer_enables_connect_protocol(&self) -> bool {
        self.peer_settings.enable_connect_protocol()
    }

    /// Open an HTTP/2 extended CONNECT (RFC 8441) bidirectional stream.
    ///
    /// On success, returns an [`H2ConnectStream`] that implements
    /// `AsyncRead + AsyncWrite` over the pooled connection. Inbound
    /// DATA frames are surfaced as bytes to the reader; writes are
    /// chunked into DATA frames that honour flow control. Dropping
    /// the returned stream emits an END_STREAM DATA frame.
    ///
    /// Errors immediately if the peer has not advertised
    /// [`SETTINGS_ENABLE_CONNECT_PROTOCOL`](
    /// crate::config::SETTINGS_ENABLE_CONNECT_PROTOCOL). Callers should
    /// fall back to the HTTP/1.1 WebSocket path in that case.
    pub async fn open_extended_connect(
        &self,
        pseudo: PseudoHeaders,
        headers: Vec<(String, String)>,
    ) -> Result<H2ConnectStream, H2Error> {
        if !self.peer_enables_connect_protocol() {
            return Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: "peer did not advertise SETTINGS_ENABLE_CONNECT_PROTOCOL=1; \
                         fall back to the HTTP/1.1 path"
                    .into(),
            });
        }
        if self.closed.load(Ordering::Acquire) {
            return Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "connection closed".into(),
            });
        }
        if !pseudo.method.eq_ignore_ascii_case("CONNECT") || pseudo.protocol.is_none() {
            return Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: "extended CONNECT requires :method=CONNECT and :protocol".into(),
            });
        }

        let (write_tx, write_rx) = mpsc::channel::<io::Result<Bytes>>(STREAM_REQ_BODY_CAPACITY);
        let (body_tx, body_rx) = mpsc::channel::<io::Result<Bytes>>(STREAM_RESP_BODY_CAPACITY);
        let (headers_tx, headers_rx) = oneshot::channel::<Result<H2ResponseEx, H2Error>>();

        let cmd = DriverCommand::OpenConnect {
            pseudo,
            headers,
            write_rx,
            headers_tx,
            body_tx,
        };
        self.tx.send(cmd).await.map_err(|_| H2Error::Connection {
            code: ErrorCode::NoError,
            reason: "driver task has exited".into(),
        })?;

        let resp = match headers_rx.await {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(H2Error::Connection {
                    code: ErrorCode::NoError,
                    reason: "driver dropped response sender".into(),
                });
            }
        };

        Ok(H2ConnectStream {
            status: resp.status,
            response_headers: resp.headers,
            write_tx: Some(write_tx),
            read_rx: body_rx,
            read_leftover: Bytes::new(),
            read_eof: false,
        })
    }
}

/// Bidirectional stream over an HTTP/2 connection opened via
/// RFC 8441 extended CONNECT.
///
/// Implements [`AsyncRead`](tokio::io::AsyncRead) and
/// [`AsyncWrite`](tokio::io::AsyncWrite) so higher layers
/// (tokio-tungstenite, an arbitrary framed protocol) can run on top.
/// Writes are chunked through the H2 driver's flow-control machinery;
/// reads drain DATA frames the driver pushes into the inbound channel.
/// Dropping the stream closes the write half gracefully with an
/// END_STREAM DATA frame.
pub struct H2ConnectStream {
    status: u16,
    response_headers: Vec<(String, String)>,
    /// Wrapped in `Option` so `poll_shutdown` and `Drop` can take it
    /// to signal EOF to the driver-side relay task.
    write_tx: Option<mpsc::Sender<io::Result<Bytes>>>,
    read_rx: mpsc::Receiver<io::Result<Bytes>>,
    read_leftover: Bytes,
    read_eof: bool,
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
        let tx = match this.write_tx.as_ref() {
            Some(t) => t,
            None => {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "H2ConnectStream write half closed",
                )));
            }
        };
        match tx.try_reserve() {
            Ok(permit) => {
                let n = buf.len();
                permit.send(Ok(Bytes::copy_from_slice(buf)));
                Poll::Ready(Ok(n))
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "H2 driver dropped the CONNECT stream",
            ))),
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
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        self.write_tx = None;
        std::task::Poll::Ready(Ok(()))
    }
}

impl Drop for H2ConnectStream {
    fn drop(&mut self) {
        self.write_tx = None;
    }
}

/// Driver task handle. Dropping it does **not** stop the driver — drop
/// all [`H2Client`] handles for graceful shutdown. This handle exists so
/// callers can `await` the driver's final status or force-abort it.
pub struct DriverTask {
    join: JoinHandle<Result<(), H2Error>>,
}

impl DriverTask {
    /// Wait for the driver to finish and return its final status.
    pub async fn join(self) -> Result<(), H2Error> {
        match self.join.await {
            Ok(res) => res,
            Err(e) if e.is_cancelled() => Ok(()),
            Err(e) => Err(H2Error::Connection {
                code: ErrorCode::InternalError,
                reason: format!("driver task panicked: {e}"),
            }),
        }
    }

    /// Abort the driver task forcibly. Prefer dropping all handles.
    pub fn abort(self) {
        self.join.abort();
    }
}

/// Request body shape as seen by the driver. The user-provided `Stream`
/// is converted into an mpsc receiver before the command is enqueued.
pub(crate) enum DriverRequestBody {
    None,
    Buffered(Bytes),
    Streaming {
        rx: mpsc::Receiver<io::Result<Bytes>>,
        /// Retained for future use — H2 doesn't need content-length at
        /// the frame layer, but downstream consumers may want to know
        /// the declared size.
        #[allow(dead_code)]
        length_hint: Option<u64>,
    },
}

/// Commands the driver accepts from handles.
pub(crate) enum DriverCommand {
    SendRequest {
        pseudo: PseudoHeaders,
        headers: Vec<(String, String)>,
        body: Option<Bytes>,
        /// Empty = no trailers.
        trailers: Vec<(String, String)>,
        response_tx: oneshot::Sender<Result<H2Response, H2Error>>,
    },
    SendRequestEx {
        pseudo: PseudoHeaders,
        headers: Vec<(String, String)>,
        body: DriverRequestBody,
        stream_response: bool,
        response_tx: oneshot::Sender<Result<H2ResponseEx, H2Error>>,
        /// For streaming-response mode, the sender side of the body
        /// chunk channel. Ignored in buffered-response mode. The
        /// caller keeps the receiver and stitches it into the returned
        /// `H2ResponseEx` after receiving headers.
        stream_body_tx: mpsc::Sender<io::Result<Bytes>>,
    },
    /// Open an RFC 8441 extended CONNECT stream that stays
    /// bidirectional until the caller drops the
    /// [`H2ConnectStream`] handle or the peer tears it down. The
    /// request HEADERS carries no END_STREAM flag, so DATA frames
    /// flow in both directions for the lifetime of the stream.
    OpenConnect {
        pseudo: PseudoHeaders,
        headers: Vec<(String, String)>,
        /// Outbound (user-side) data: a relay task forwards chunks
        /// read from here into the driver's internal body-chunk
        /// channel, which drives DATA-frame emission.
        write_rx: mpsc::Receiver<io::Result<Bytes>>,
        /// Delivered once :status HEADERS arrive — callers observe
        /// the WebSocket handshake outcome synchronously before
        /// receiving the stream handle.
        headers_tx: oneshot::Sender<Result<H2ResponseEx, H2Error>>,
        /// Inbound DATA chunks — driver forwards here, caller reads
        /// them through the [`H2ConnectStream`] `AsyncRead` impl.
        body_tx: mpsc::Sender<io::Result<Bytes>>,
    },
}

/// Pump a user `Stream<Item = io::Result<Bytes>>` into an mpsc the driver
/// can `recv` from. Runs in its own task so the driver's single-task
/// invariant is preserved.
async fn pump_request_body(
    mut stream: Pin<Box<dyn futures_util::Stream<Item = io::Result<Bytes>> + Send + 'static>>,
    tx: mpsc::Sender<io::Result<Bytes>>,
) {
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let is_err = chunk.is_err();
        if tx.send(chunk).await.is_err() {
            // Receiver dropped — driver failed or already finished.
            return;
        }
        if is_err {
            return;
        }
    }
    // Drop tx to signal EOF.
}

/// Per-stream relay: forwards chunks from the caller-owned `rx` into
/// the driver-wide `chunk_tx`, tagging each message with `stream_id`.
async fn relay_request_body(
    stream_id: u32,
    mut rx: mpsc::Receiver<io::Result<Bytes>>,
    chunk_tx: mpsc::Sender<BodyChunkIn>,
) {
    while let Some(item) = rx.recv().await {
        match item {
            Ok(data) => {
                if data.is_empty() {
                    continue;
                }
                if chunk_tx
                    .send(BodyChunkIn::Chunk { stream_id, data })
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Err(e) => {
                let _ = chunk_tx
                    .send(BodyChunkIn::Eof {
                        stream_id,
                        error: Some(e),
                    })
                    .await;
                return;
            }
        }
    }
    let _ = chunk_tx
        .send(BodyChunkIn::Eof {
            stream_id,
            error: None,
        })
        .await;
}

/// Spawn a driver task over the given IO, after performing the HTTP/2
/// handshake (preface + SETTINGS exchange).
pub(crate) async fn start<T>(
    io: T,
    config: H2Config,
) -> Result<(H2Client, DriverTask), H2Error>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (read_half, write_half) = tokio::io::split(io);
    let mut reader = FrameReader::new(read_half);
    let mut writer = FrameWriter::new(write_half);

    // 1. Preface.
    writer.write_preface().await?;

    // 2. Our SETTINGS (ordered per fingerprint config).
    let settings_frame = SettingsFrame {
        ack: false,
        params: config
            .settings
            .iter()
            .map(|(id, val)| (id_to_u16(id), *val))
            .collect(),
    };
    writer.write_settings(&settings_frame).await?;

    // 3. WINDOW_UPDATE if connection window > default 65535.
    let default_window: u32 = 65535;
    if config.initial_connection_window_size > default_window {
        let increment = config.initial_connection_window_size - default_window;
        writer
            .write_window_update(&WindowUpdateFrame {
                stream_id: 0,
                increment,
            })
            .await?;
    }
    writer.flush().await?;

    // 4. Read the server's SETTINGS and wait for the server's ACK of
    //    our own SETTINGS (RFC 9113 §6.5.3: SETTINGS_TIMEOUT if the
    //    peer fails to ACK within a reasonable window).
    let mut peer_settings = PeerSettings::default();
    let mut got_settings = false;
    let mut our_settings_acked = false;
    let mut initial_send_window: i64 = 65535;
    let deadline = tokio::time::Instant::now() + config.settings_ack_timeout;

    while !(got_settings && our_settings_acked) {
        let next = match tokio::time::timeout_at(deadline, reader.next()).await {
            Ok(inner) => inner?,
            Err(_) => {
                return Err(H2Error::Connection {
                    code: ErrorCode::SettingsTimeout,
                    reason: format!(
                        "peer did not ACK our SETTINGS within {:?}",
                        config.settings_ack_timeout
                    ),
                });
            }
        };
        let frame = next.ok_or_else(|| H2Error::Connection {
            code: ErrorCode::ProtocolError,
            reason: "connection closed before SETTINGS".into(),
        })?;

        match frame {
            Frame::Settings(s) if !s.ack => {
                let _result = peer_settings.apply(&s.params)?;
                writer.write_settings_ack().await?;
                writer.flush().await?;
                got_settings = true;
            }
            Frame::Settings(s) if s.ack => {
                our_settings_acked = true;
            }
            Frame::WindowUpdate(w) if w.stream_id == 0 => {
                initial_send_window += w.increment as i64;
            }
            Frame::WindowUpdate(_) => {}
            Frame::GoAway(g) => {
                return Err(H2Error::Connection {
                    code: g.error_code,
                    reason: format!("server sent GOAWAY during handshake: {:?}", g.error_code),
                });
            }
            _ => {}
        }
    }

    reader.set_max_frame_size(peer_settings.max_frame_size);

    // HPACK codecs.
    let encoder = hpack::Encoder::new();
    let mut decoder = hpack::Decoder::new();
    let max_hdr = config
        .settings
        .iter()
        .find(|(id, _)| matches!(id, crate::config::SettingId::MaxHeaderListSize))
        .map(|(_, v)| *v as usize)
        .unwrap_or(256 * 1024);
    decoder.set_max_header_list_size(max_hdr);
    decoder.set_max_table_size(peer_settings.header_table_size as usize);

    // Publish snapshot.
    let snapshot = Arc::new(PeerSettingsSnapshot::new());
    snapshot.set_max_concurrent_streams(peer_settings.max_concurrent_streams);
    snapshot.set_enable_connect_protocol(peer_settings.enable_connect_protocol);

    let (tx, rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
    let closed = Arc::new(AtomicBool::new(false));

    let (body_chunk_tx, body_chunk_rx) = mpsc::channel(STREAM_REQ_BODY_CAPACITY * 4);

    let driver = Driver {
        reader,
        writer,
        encoder,
        decoder,
        peer_settings,
        peer_snapshot: snapshot.clone(),
        conn_send_window: initial_send_window,
        conn_recv_window: config.initial_connection_window_size as i64,
        streams: HashMap::new(),
        next_stream_id: 1,
        buffered_pending: VecDeque::new(),
        rst_flood: RstFloodDetector::new(
            config.rst_stream_flood_threshold,
            config.rst_stream_flood_window,
        ),
        config: config.clone(),
        command_rx: rx,
        closed: closed.clone(),
        peer_goaway_last_stream: None,
        shutdown_started: false,
        body_chunk_tx,
        body_chunk_rx,
    };

    let join = tokio::spawn(driver.run());

    Ok((
        H2Client {
            tx,
            closed,
            peer_settings: snapshot,
        },
        DriverTask { join },
    ))
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
    fn new(
        send_window: i64,
        recv_window: i64,
        sink: ResponseSink,
        drop_body: bool,
    ) -> Self {
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
                    let _ = body_tx
                        .try_send(Err(io::Error::other(format!("h2 stream failed: {err}"))));
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

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    async fn run(mut self) -> Result<(), H2Error> {
        let result = self.event_loop().await;
        // Mark closed so handles stop enqueuing new commands.
        self.closed.store(true, Ordering::Release);
        // Fail any remaining pending requests with the final status.
        let final_err = match &result {
            Ok(()) => H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "connection closed".into(),
            },
            Err(e) => clone_err(e),
        };
        for (_, mut actor) in self.streams.drain() {
            actor.deliver_err(clone_err(&final_err));
        }
        // Drain remaining commands in the channel and fail them.
        while let Ok(cmd) = self.command_rx.try_recv() {
            match cmd {
                DriverCommand::SendRequest { response_tx, .. } => {
                    let _ = response_tx.send(Err(clone_err(&final_err)));
                }
                DriverCommand::SendRequestEx { response_tx, .. } => {
                    let _ = response_tx.send(Err(clone_err(&final_err)));
                }
                DriverCommand::OpenConnect { headers_tx, .. } => {
                    let _ = headers_tx.send(Err(clone_err(&final_err)));
                }
            }
        }
        result
    }

    async fn event_loop(&mut self) -> Result<(), H2Error> {
        loop {
            tokio::select! {
                biased;
                frame = self.reader.next() => {
                    match frame? {
                        Some(f) => self.on_inbound_frame(f).await?,
                        None => {
                            // Reader EOF. If we've initiated shutdown, this is ok.
                            return if self.shutdown_started {
                                Ok(())
                            } else {
                                Err(H2Error::Connection {
                                    code: ErrorCode::NoError,
                                    reason: "peer closed connection".into(),
                                })
                            };
                        }
                    }
                }
                maybe_cmd = self.command_rx.recv() => {
                    match maybe_cmd {
                        Some(cmd) => self.on_command(cmd).await?,
                        None => {
                            // Last handle dropped — graceful shutdown.
                            return self.graceful_shutdown().await;
                        }
                    }
                }
                maybe_chunk = self.body_chunk_rx.recv() => {
                    if let Some(c) = maybe_chunk {
                        self.on_body_chunk(c).await?;
                    }
                }
            }

            // After any event we try to drain pending sends because the
            // write window may have grown (WINDOW_UPDATE / SETTINGS).
            self.try_drain_pending().await?;
        }
    }

    // -------------------------------------------------------------------
    // Command handling
    // -------------------------------------------------------------------

    async fn on_command(&mut self, cmd: DriverCommand) -> Result<(), H2Error> {
        match cmd {
            DriverCommand::SendRequest {
                pseudo,
                headers,
                body,
                trailers,
                response_tx,
            } => {
                if self.peer_goaway_last_stream.is_some() {
                    let _ = response_tx.send(Err(H2Error::Connection {
                        code: ErrorCode::NoError,
                        reason: "peer sent GOAWAY, refusing new streams".into(),
                    }));
                    return Ok(());
                }
                if let Some(limit) = self.peer_settings.max_concurrent_streams {
                    if self.active_stream_count() >= limit {
                        let _ = response_tx.send(Err(H2Error::Connection {
                            code: ErrorCode::RefusedStream,
                            reason: "MAX_CONCURRENT_STREAMS exceeded".into(),
                        }));
                        return Ok(());
                    }
                }
                if self.next_stream_id > 0x7FFF_FFFF {
                    let _ = response_tx.send(Err(H2Error::Connection {
                        code: ErrorCode::NoError,
                        reason: "stream ID space exhausted".into(),
                    }));
                    return Ok(());
                }
                self.start_request(
                    pseudo,
                    headers,
                    body,
                    trailers,
                    ResponseSink::Buffered(response_tx),
                )
                .await
            }
            DriverCommand::OpenConnect {
                pseudo,
                headers,
                write_rx,
                headers_tx,
                body_tx,
            } => {
                if self.peer_goaway_last_stream.is_some() {
                    let _ = headers_tx.send(Err(H2Error::Connection {
                        code: ErrorCode::NoError,
                        reason: "peer sent GOAWAY, refusing new streams".into(),
                    }));
                    return Ok(());
                }
                if !self.peer_settings.enable_connect_protocol {
                    let _ = headers_tx.send(Err(H2Error::Connection {
                        code: ErrorCode::ProtocolError,
                        reason: "peer does not advertise ENABLE_CONNECT_PROTOCOL".into(),
                    }));
                    return Ok(());
                }
                if let Some(limit) = self.peer_settings.max_concurrent_streams {
                    if self.active_stream_count() >= limit {
                        let _ = headers_tx.send(Err(H2Error::Connection {
                            code: ErrorCode::RefusedStream,
                            reason: "MAX_CONCURRENT_STREAMS exceeded".into(),
                        }));
                        return Ok(());
                    }
                }
                if self.next_stream_id > 0x7FFF_FFFF {
                    let _ = headers_tx.send(Err(H2Error::Connection {
                        code: ErrorCode::NoError,
                        reason: "stream ID space exhausted".into(),
                    }));
                    return Ok(());
                }
                let sink = ResponseSink::StreamingEx {
                    headers_tx: Some(headers_tx),
                    body_tx,
                };
                self.start_extended_connect(pseudo, headers, write_rx, sink)
                    .await
            }
            DriverCommand::SendRequestEx {
                pseudo,
                headers,
                body,
                stream_response,
                response_tx,
                stream_body_tx,
            } => {
                if self.peer_goaway_last_stream.is_some() {
                    let _ = response_tx.send(Err(H2Error::Connection {
                        code: ErrorCode::NoError,
                        reason: "peer sent GOAWAY, refusing new streams".into(),
                    }));
                    return Ok(());
                }
                if let Some(limit) = self.peer_settings.max_concurrent_streams {
                    if self.active_stream_count() >= limit {
                        let _ = response_tx.send(Err(H2Error::Connection {
                            code: ErrorCode::RefusedStream,
                            reason: "MAX_CONCURRENT_STREAMS exceeded".into(),
                        }));
                        return Ok(());
                    }
                }
                if self.next_stream_id > 0x7FFF_FFFF {
                    let _ = response_tx.send(Err(H2Error::Connection {
                        code: ErrorCode::NoError,
                        reason: "stream ID space exhausted".into(),
                    }));
                    return Ok(());
                }
                let sink = if stream_response {
                    ResponseSink::StreamingEx {
                        headers_tx: Some(response_tx),
                        body_tx: stream_body_tx,
                    }
                } else {
                    ResponseSink::BufferedEx(response_tx)
                };
                self.start_request_ex(pseudo, headers, body, sink).await
            }
        }
    }

    async fn start_request(
        &mut self,
        pseudo: PseudoHeaders,
        headers: Vec<(String, String)>,
        body: Option<Bytes>,
        trailers: Vec<(String, String)>,
        sink: ResponseSink,
    ) -> Result<(), H2Error> {
        let stream_id = self.next_stream_id;
        self.next_stream_id = stream_id + 2;
        let is_head = pseudo.method.eq_ignore_ascii_case("HEAD");

        // Build pseudo-header list (CONNECT aware).
        let header_list = match pseudo.build_pseudo_list(&self.config.pseudo_order) {
            Ok(list) => list,
            Err(e) => {
                send_err_to_sink(sink, e);
                return Ok(());
            }
        };
        // Encode header block.
        let fragment = encode_request_pseudos(&mut self.encoder, header_list, &headers);

        let has_trailers = !trailers.is_empty();
        let end_stream_on_headers = body.is_none() && !has_trailers;

        let initial_send = self.peer_settings.initial_window_size as i64;
        let initial_recv = self.config.initial_connection_window_size as i64;
        let mut actor = StreamActor::new(initial_send, initial_recv, sink, is_head);

        // Drive state machine: SendHeaders.
        if let Err(e) = actor.state.transition(StreamEvent::SendHeaders {
            end_stream: end_stream_on_headers,
        }) {
            let err = map_state_err(stream_id, e);
            actor.deliver_err(err);
            return Ok(());
        }

        // Insert actor now so inbound frames can find it.
        self.streams.insert(stream_id, actor);

        // Write HEADERS (+ CONTINUATION if needed).
        if let Err(e) = self
            .write_headers_block(stream_id, end_stream_on_headers, fragment, true)
            .await
        {
            self.fail_stream(stream_id, e);
            return Ok(());
        }

        // Handle body (if any). `write_body_or_park` emits trailing HEADERS
        // on completion, so when a body is present we're done after the call.
        let had_body = body.is_some();
        if let Some(body) = body {
            if let Err(e) = self
                .write_body_or_park(stream_id, body, has_trailers, trailers.clone())
                .await
            {
                self.fail_stream(stream_id, e);
                return Ok(());
            }
        }

        // Body-less request with trailers: HEADERS didn't carry END_STREAM,
        // so we emit the trailer block directly.
        if !had_body && has_trailers {
            if let Err(e) = self.write_trailers(stream_id, trailers).await {
                self.fail_stream(stream_id, e);
                return Ok(());
            }
        }

        // Flush best-effort.
        let _ = self.writer.flush().await;

        Ok(())
    }

    async fn start_request_ex(
        &mut self,
        pseudo: PseudoHeaders,
        headers: Vec<(String, String)>,
        body: DriverRequestBody,
        sink: ResponseSink,
    ) -> Result<(), H2Error> {
        // Buffered / None bodies delegate to the legacy start_request
        // by wrapping into `Option<Bytes>` — same framing, same logic.
        match body {
            DriverRequestBody::None => {
                self.start_request(pseudo, headers, None, Vec::new(), sink)
                    .await
            }
            DriverRequestBody::Buffered(b) => {
                let body = if b.is_empty() { None } else { Some(b) };
                self.start_request(pseudo, headers, body, Vec::new(), sink)
                    .await
            }
            DriverRequestBody::Streaming { rx, length_hint: _ } => {
                // Streaming body. Allocate a stream id, send HEADERS
                // without END_STREAM, install the actor with a
                // `SendBodyInput::Streaming`, then spawn a relay task
                // that forwards chunks from `rx` into the driver's
                // shared `body_chunk_tx`. The event loop pulls chunks
                // and feeds them into `write_body_or_park`.
                let stream_id = self.next_stream_id;
                self.next_stream_id = stream_id + 2;
                let is_head = pseudo.method.eq_ignore_ascii_case("HEAD");

                let header_list = match pseudo.build_pseudo_list(&self.config.pseudo_order) {
                    Ok(list) => list,
                    Err(e) => {
                        send_err_to_sink(sink, e);
                        return Ok(());
                    }
                };
                let fragment = encode_request_pseudos(&mut self.encoder, header_list, &headers);

                let initial_send = self.peer_settings.initial_window_size as i64;
                let initial_recv = self.config.initial_connection_window_size as i64;
                let mut actor = StreamActor::new(initial_send, initial_recv, sink, is_head);
                actor.send_body_input = SendBodyInput::Streaming {
                    pending_buf: VecDeque::new(),
                    closed: false,
                    error: None,
                    trailers: Vec::new(),
                };

                if let Err(e) =
                    actor.state.transition(StreamEvent::SendHeaders { end_stream: false })
                {
                    let err = map_state_err(stream_id, e);
                    actor.deliver_err(err);
                    return Ok(());
                }

                self.streams.insert(stream_id, actor);

                if let Err(e) = self
                    .write_headers_block(stream_id, false, fragment, true)
                    .await
                {
                    self.fail_stream(stream_id, e);
                    return Ok(());
                }

                // Spawn relay: reads chunks from caller rx, forwards
                // them through the driver-shared mpsc tagged by stream
                // id. Driver's event_loop handles them in a select
                // branch.
                let chunk_tx = self.body_chunk_tx.clone();
                tokio::spawn(relay_request_body(stream_id, rx, chunk_tx));

                // We stop here; inbound chunk events will arrive via
                // `BodyChunkIn` and trigger further writes.
                let _ = self.writer.flush().await;
                Ok(())
            }
        }
    }

    /// Open an RFC 8441 extended CONNECT stream. Shape mirrors the
    /// streaming-body path from [`start_request_ex`](Self::start_request_ex),
    /// but:
    ///
    /// - the HEADERS frame never carries END_STREAM — the stream is
    ///   bidirectional until the peer/caller tears it down,
    /// - the response sink is always streaming so the caller can start
    ///   draining inbound DATA frames as soon as HEADERS arrive.
    ///
    /// Pseudo-header validation (`:method=CONNECT`, `:protocol` present)
    /// already happened on the handle before the command reached here.
    async fn start_extended_connect(
        &mut self,
        pseudo: PseudoHeaders,
        headers: Vec<(String, String)>,
        write_rx: mpsc::Receiver<io::Result<Bytes>>,
        sink: ResponseSink,
    ) -> Result<(), H2Error> {
        let stream_id = self.next_stream_id;
        self.next_stream_id = stream_id + 2;

        let header_list = match pseudo.build_pseudo_list(&self.config.pseudo_order) {
            Ok(list) => list,
            Err(e) => {
                send_err_to_sink(sink, e);
                return Ok(());
            }
        };
        let fragment = encode_request_pseudos(&mut self.encoder, header_list, &headers);

        let initial_send = self.peer_settings.initial_window_size as i64;
        let initial_recv = self.config.initial_connection_window_size as i64;
        let mut actor = StreamActor::new(initial_send, initial_recv, sink, false);
        actor.send_body_input = SendBodyInput::Streaming {
            pending_buf: VecDeque::new(),
            closed: false,
            error: None,
            trailers: Vec::new(),
        };

        if let Err(e) = actor
            .state
            .transition(StreamEvent::SendHeaders { end_stream: false })
        {
            let err = map_state_err(stream_id, e);
            actor.deliver_err(err);
            return Ok(());
        }

        self.streams.insert(stream_id, actor);

        if let Err(e) = self
            .write_headers_block(stream_id, false, fragment, true)
            .await
        {
            self.fail_stream(stream_id, e);
            return Ok(());
        }

        let chunk_tx = self.body_chunk_tx.clone();
        tokio::spawn(relay_request_body(stream_id, write_rx, chunk_tx));

        let _ = self.writer.flush().await;
        Ok(())
    }

    async fn on_body_chunk(&mut self, chunk: BodyChunkIn) -> Result<(), H2Error> {
        match chunk {
            BodyChunkIn::Chunk { stream_id, data } => {
                let should_try_write = if let Some(actor) = self.streams.get_mut(&stream_id) {
                    if let SendBodyInput::Streaming { pending_buf, .. } =
                        &mut actor.send_body_input
                    {
                        pending_buf.push_back(data);
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                if should_try_write {
                    self.try_pump_streaming_body(stream_id).await?;
                }
            }
            BodyChunkIn::Eof { stream_id, error } => {
                let should_try_write = if let Some(actor) = self.streams.get_mut(&stream_id) {
                    if let SendBodyInput::Streaming {
                        closed,
                        error: e,
                        ..
                    } = &mut actor.send_body_input
                    {
                        *closed = true;
                        if let Some(err) = error {
                            *e = Some(err);
                        }
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                if should_try_write {
                    self.try_pump_streaming_body(stream_id).await?;
                }
            }
        }
        Ok(())
    }

    /// Drain as much of the per-stream streaming body buffer as the flow
    /// control window permits; emit DATA frames accordingly. Marks
    /// END_STREAM on the final frame when the producer has EOF'd.
    async fn try_pump_streaming_body(&mut self, stream_id: u32) -> Result<(), H2Error> {
        loop {
            // Take the next chunk out of the actor's buffer (if any).
            let next_chunk: Option<Bytes> = self
                .streams
                .get_mut(&stream_id)
                .and_then(|a| match &mut a.send_body_input {
                    SendBodyInput::Streaming { pending_buf, .. } => pending_buf.pop_front(),
                    _ => None,
                });

            let (closed, stream_err) = self
                .streams
                .get(&stream_id)
                .and_then(|a| match &a.send_body_input {
                    SendBodyInput::Streaming { closed, error, .. } => {
                        Some((*closed, error.as_ref().map(|e| e.to_string())))
                    }
                    _ => None,
                })
                .unwrap_or((false, None));

            let already_closed = self
                .streams
                .get(&stream_id)
                .map(|a| a.send_closed)
                .unwrap_or(true);
            if already_closed {
                return Ok(());
            }

            if stream_err.is_some() && next_chunk.is_none() {
                // Cancel the stream — producer errored.
                let _ = self
                    .writer
                    .write_rst_stream(stream_id, ErrorCode::InternalError)
                    .await;
                let err = H2Error::Stream {
                    stream_id,
                    code: ErrorCode::InternalError,
                };
                self.fail_stream(stream_id, err);
                return Ok(());
            }

            match next_chunk {
                Some(chunk) if !chunk.is_empty() => {
                    // Write as much as possible, park on flow control.
                    self.write_streaming_chunk(stream_id, chunk, closed).await?;
                    // If we parked (pending_send set), stop pumping —
                    // resumption happens via try_drain_pending.
                    let parked = self
                        .streams
                        .get(&stream_id)
                        .map(|a| a.pending_send.is_some())
                        .unwrap_or(false);
                    if parked {
                        return Ok(());
                    }
                }
                _ => {
                    if closed {
                        // No more chunks and producer closed: emit an
                        // empty END_STREAM DATA frame if we haven't yet.
                        if !already_closed {
                            if let Some(actor) = self.streams.get_mut(&stream_id) {
                                if let Err(e) = actor
                                    .state
                                    .transition(StreamEvent::SendData { end_stream: true })
                                {
                                    return Err(map_state_err(stream_id, e));
                                }
                                actor.send_closed = true;
                            }
                            self.writer
                                .write_data(&DataFrame {
                                    stream_id,
                                    end_stream: true,
                                    data: Bytes::new(),
                                })
                                .await?;
                            let _ = self.writer.flush().await;
                        }
                    }
                    return Ok(());
                }
            }
        }
    }

    async fn write_streaming_chunk(
        &mut self,
        stream_id: u32,
        mut chunk: Bytes,
        producer_closed: bool,
    ) -> Result<(), H2Error> {
        while !chunk.is_empty() {
            let (conn_avail, stream_avail) = {
                let conn = self.conn_send_window.max(0) as usize;
                let stream_win = self
                    .streams
                    .get(&stream_id)
                    .map(|i| i.send_window.max(0) as usize)
                    .unwrap_or(0);
                (conn, stream_win)
            };
            let window = conn_avail.min(stream_avail);
            if window == 0 {
                // Park — stash the remainder in `pending_send`. The
                // next WINDOW_UPDATE will resume via try_drain_pending.
                if let Some(actor) = self.streams.get_mut(&stream_id) {
                    actor.pending_send = Some(PendingSend {
                        remaining: chunk.clone(),
                        trailers: Vec::new(),
                    });
                    if !self.buffered_pending.contains(&stream_id) {
                        self.buffered_pending.push_back(stream_id);
                    }
                }
                return Ok(());
            }
            let max_frame = self.peer_settings.max_frame_size as usize;
            let chunk_size = chunk.len().min(max_frame).min(window);
            let piece = chunk.slice(0..chunk_size);
            chunk = chunk.slice(chunk_size..);

            // Is this the last DATA frame? Only if producer has EOF'd
            // AND no more buffered chunks follow.
            let no_more_buffered = self
                .streams
                .get(&stream_id)
                .map(|a| match &a.send_body_input {
                    SendBodyInput::Streaming { pending_buf, .. } => pending_buf.is_empty(),
                    _ => true,
                })
                .unwrap_or(true);
            let is_last = chunk.is_empty() && producer_closed && no_more_buffered;

            if let Some(actor) = self.streams.get_mut(&stream_id) {
                if let Err(e) = actor
                    .state
                    .transition(StreamEvent::SendData { end_stream: is_last })
                {
                    return Err(map_state_err(stream_id, e));
                }
                if is_last {
                    actor.send_closed = true;
                }
            }

            self.writer
                .write_data(&DataFrame {
                    stream_id,
                    end_stream: is_last,
                    data: piece,
                })
                .await?;

            self.conn_send_window -= chunk_size as i64;
            if let Some(actor) = self.streams.get_mut(&stream_id) {
                actor.send_window -= chunk_size as i64;
            }
            if is_last {
                let _ = self.writer.flush().await;
            }
        }
        Ok(())
    }

    async fn write_headers_block(
        &mut self,
        stream_id: u32,
        end_stream: bool,
        fragment: Vec<u8>,
        with_priority: bool,
    ) -> Result<(), H2Error> {
        let max_frame = self.peer_settings.max_frame_size as usize;
        let priority = if with_priority {
            self.config.default_priority.map(|p| StreamDependency {
                exclusive: p.exclusive,
                dependency_id: p.stream_dependency,
                weight: p.weight,
            })
        } else {
            None
        };
        let prio_overhead = if priority.is_some() { 5 } else { 0 };

        if fragment.len() + prio_overhead <= max_frame {
            self.writer
                .write_headers(&HeadersFrame {
                    stream_id,
                    end_stream,
                    end_headers: true,
                    priority,
                    fragment: Bytes::from(fragment),
                })
                .await?;
        } else {
            let first_cap = max_frame.saturating_sub(prio_overhead).max(1);
            let first_len = first_cap.min(fragment.len());
            let first = &fragment[..first_len];
            self.writer
                .write_headers(&HeadersFrame {
                    stream_id,
                    end_stream,
                    end_headers: false,
                    priority,
                    fragment: Bytes::copy_from_slice(first),
                })
                .await?;
            let mut offset = first_len;
            while offset < fragment.len() {
                let end = (offset + max_frame).min(fragment.len());
                let is_last = end == fragment.len();
                let chunk = &fragment[offset..end];
                let mut buf = BytesMut::with_capacity(9 + chunk.len());
                let header = crate::frame::FrameHeader {
                    length: chunk.len() as u32,
                    frame_type: 0x9,
                    flags: if is_last { 0x4 } else { 0 },
                    stream_id,
                };
                header.encode(&mut buf);
                buf.extend_from_slice(chunk);
                self.writer.write_raw(&buf).await?;
                offset = end;
            }
        }
        Ok(())
    }

    /// Try to write `body` for `stream_id`; on flow-control exhaustion,
    /// stash the remainder in the actor's pending_send and return.
    async fn write_body_or_park(
        &mut self,
        stream_id: u32,
        body: Bytes,
        has_trailers: bool,
        trailers: Vec<(String, String)>,
    ) -> Result<(), H2Error> {
        // Attempt to write as much as flow control allows.
        let mut remaining = body;
        while !remaining.is_empty() {
            let (conn_avail, stream_avail) = {
                let conn = self.conn_send_window.max(0) as usize;
                let info = self.streams.get(&stream_id);
                let stream_win = info.map(|i| i.send_window).unwrap_or(0);
                (conn, stream_win.max(0) as usize)
            };
            let window = conn_avail.min(stream_avail);
            if window == 0 {
                // Park.
                self.park_stream(
                    stream_id,
                    PendingSend {
                        remaining,
                        trailers,
                    },
                );
                return Ok(());
            }
            let max_frame = self.peer_settings.max_frame_size as usize;
            let chunk_size = remaining.len().min(max_frame).min(window);
            let chunk = remaining.slice(0..chunk_size);
            remaining = remaining.slice(chunk_size..);
            let is_last = remaining.is_empty();
            let data_end_stream = is_last && !has_trailers;

            // Drive state machine.
            if let Some(actor) = self.streams.get_mut(&stream_id) {
                if let Err(e) = actor.state.transition(StreamEvent::SendData {
                    end_stream: data_end_stream,
                }) {
                    return Err(map_state_err(stream_id, e));
                }
            }

            self.writer
                .write_data(&DataFrame {
                    stream_id,
                    end_stream: data_end_stream,
                    data: chunk,
                })
                .await?;

            self.conn_send_window -= chunk_size as i64;
            if let Some(actor) = self.streams.get_mut(&stream_id) {
                actor.send_window -= chunk_size as i64;
            }
        }

        // Body fully written. Send trailers if present.
        if has_trailers {
            self.write_trailers(stream_id, trailers).await?;
        }
        Ok(())
    }

    async fn write_trailers(
        &mut self,
        stream_id: u32,
        trailers: Vec<(String, String)>,
    ) -> Result<(), H2Error> {
        if let Some(actor) = self.streams.get_mut(&stream_id) {
            if let Err(e) = actor.state.transition(StreamEvent::SendTrailers) {
                return Err(map_state_err(stream_id, e));
            }
        }
        let mut list: Vec<(&str, &str)> = Vec::with_capacity(trailers.len());
        for (n, v) in &trailers {
            list.push((n, v));
        }
        let fragment = self.encoder.encode_header_block(&list);
        let max_frame = self.peer_settings.max_frame_size as usize;
        if fragment.len() > max_frame {
            return Err(H2Error::Connection {
                code: ErrorCode::InternalError,
                reason: "trailer block exceeds max_frame_size".into(),
            });
        }
        self.writer
            .write_headers(&HeadersFrame {
                stream_id,
                end_stream: true,
                end_headers: true,
                priority: None,
                fragment: Bytes::from(fragment),
            })
            .await?;
        Ok(())
    }

    fn park_stream(&mut self, stream_id: u32, pending: PendingSend) {
        if let Some(actor) = self.streams.get_mut(&stream_id) {
            actor.pending_send = Some(pending);
            if !self.buffered_pending.contains(&stream_id) {
                self.buffered_pending.push_back(stream_id);
            }
        }
    }

    /// Try to resume each parked stream that now has flow-control credit.
    async fn try_drain_pending(&mut self) -> Result<(), H2Error> {
        if self.buffered_pending.is_empty() {
            return Ok(());
        }
        // Drain by repeatedly popping the front; re-park if still blocked.
        let mut progress_count = self.buffered_pending.len();
        while progress_count > 0 && !self.buffered_pending.is_empty() {
            progress_count -= 1;
            let sid = match self.buffered_pending.pop_front() {
                Some(s) => s,
                None => break,
            };
            // Extract pending.
            let pending = match self.streams.get_mut(&sid).and_then(|a| a.pending_send.take()) {
                Some(p) => p,
                None => continue,
            };

            // Streaming input? Push the pending.remaining back into the
            // front of the streaming buffer and let the streaming pump
            // handle it (it understands END_STREAM timing relative to
            // the producer's EOF signal).
            let is_streaming = self
                .streams
                .get(&sid)
                .map(|a| matches!(a.send_body_input, SendBodyInput::Streaming { .. }))
                .unwrap_or(false);
            if is_streaming {
                if let Some(actor) = self.streams.get_mut(&sid) {
                    if let SendBodyInput::Streaming { pending_buf, .. } =
                        &mut actor.send_body_input
                    {
                        if !pending.remaining.is_empty() {
                            pending_buf.push_front(pending.remaining);
                        }
                    }
                }
                if let Err(e) = self.try_pump_streaming_body(sid).await {
                    self.fail_stream(sid, e);
                } else {
                    let _ = self.writer.flush().await;
                }
                continue;
            }

            let has_trailers = !pending.trailers.is_empty();
            let result = self
                .write_body_or_park(sid, pending.remaining, has_trailers, pending.trailers)
                .await;
            match result {
                Ok(()) => {
                    // If the stream is still parked, we already pushed it
                    // back via park_stream. Otherwise it's done sending.
                    let _ = self.writer.flush().await;
                }
                Err(e) => {
                    self.fail_stream(sid, e);
                }
            }
        }
        Ok(())
    }

    // -------------------------------------------------------------------
    // Inbound frame handling
    // -------------------------------------------------------------------

    async fn on_inbound_frame(&mut self, frame: Frame) -> Result<(), H2Error> {
        // RFC 9113 §5.1.1: client-initiated streams use odd identifiers;
        // server-initiated (PUSH_PROMISE) use even. Any HEADERS / DATA
        // frame from the peer on an even stream id — or on any stream
        // id the client didn't originate — is a connection error. We
        // catch this at the entry point rather than in every handler.
        let enforce_odd = |sid: u32| -> Result<(), H2Error> {
            if sid == 0 || sid % 2 == 0 {
                return Err(H2Error::Connection {
                    code: ErrorCode::ProtocolError,
                    reason: format!(
                        "peer used server-initiated stream id {sid} for a client-expected frame"
                    ),
                });
            }
            Ok(())
        };

        match frame {
            Frame::Headers(h) => {
                enforce_odd(h.stream_id)?;
                self.on_headers(h).await?
            }
            Frame::Data(d) => {
                enforce_odd(d.stream_id)?;
                self.on_data(d).await?
            }
            Frame::Settings(s) if s.ack => {
                // ACK of our settings — ignored.
            }
            Frame::Settings(s) => {
                let result = self.peer_settings.apply(&s.params)?;
                if let Some(delta) = result.window_size_delta {
                    for actor in self.streams.values_mut() {
                        actor.send_window += delta;
                    }
                }
                self.peer_snapshot
                    .set_max_concurrent_streams(self.peer_settings.max_concurrent_streams);
                self.peer_snapshot
                    .set_enable_connect_protocol(self.peer_settings.enable_connect_protocol);
                self.writer.write_settings_ack().await?;
                self.reader
                    .set_max_frame_size(self.peer_settings.max_frame_size);
                self.decoder
                    .set_max_table_size(self.peer_settings.header_table_size as usize);
                self.encoder
                    .set_max_table_size(self.peer_settings.header_table_size as usize);
            }
            Frame::WindowUpdate(w) if w.stream_id == 0 => {
                self.conn_send_window += w.increment as i64;
            }
            Frame::WindowUpdate(w) => {
                if let Some(actor) = self.streams.get_mut(&w.stream_id) {
                    actor.send_window += w.increment as i64;
                }
            }
            Frame::Ping(p) if !p.ack => {
                self.writer.write_ping_ack(p.payload).await?;
            }
            Frame::Ping(_) => {}
            Frame::GoAway(g) => {
                // Mark connection as no-new-streams; existing streams
                // ≤ last_stream_id may finish. If error is non-zero,
                // tear the connection down as a connection error.
                self.peer_goaway_last_stream = Some(g.last_stream_id);
                if !matches!(g.error_code, ErrorCode::NoError) {
                    // Fail all streams above last_stream_id immediately.
                    let to_fail: Vec<u32> = self
                        .streams
                        .keys()
                        .copied()
                        .filter(|sid| *sid > g.last_stream_id)
                        .collect();
                    for sid in to_fail {
                        self.fail_stream(
                            sid,
                            H2Error::Connection {
                                code: g.error_code,
                                reason: format!("peer GOAWAY: {:?}", g.error_code),
                            },
                        );
                    }
                    return Err(H2Error::Connection {
                        code: g.error_code,
                        reason: format!("server sent GOAWAY: {:?}", g.error_code),
                    });
                }
            }
            Frame::RstStream(r) => {
                self.rst_flood.record(Instant::now())?;
                if let Some(actor) = self.streams.get_mut(&r.stream_id) {
                    let _ = actor
                        .state
                        .transition(StreamEvent::RecvRstStream(r.error_code));
                }
                let err = H2Error::Stream {
                    stream_id: r.stream_id,
                    code: r.error_code,
                };
                self.fail_stream(r.stream_id, err);
            }
            Frame::PushPromise(pp) => {
                self.writer
                    .write_rst_stream(pp.promised_stream_id, ErrorCode::Cancel)
                    .await?;
            }
            Frame::Continuation { .. } => {
                // A bare CONTINUATION without a preceding HEADERS we
                // already consumed is a protocol error.
                return Err(H2Error::Connection {
                    code: ErrorCode::ProtocolError,
                    reason: "unexpected CONTINUATION".into(),
                });
            }
            _ => {}
        }
        Ok(())
    }

    async fn on_headers(&mut self, h: HeadersFrame) -> Result<(), H2Error> {
        // Reassemble CONTINUATION.
        let full_fragment = if h.end_headers {
            h.fragment
        } else {
            let max_header_block = self.config.max_header_block_bytes;
            let mut assembled = h.fragment.to_vec();
            loop {
                if assembled.len() > max_header_block {
                    return Err(H2Error::Connection {
                        code: ErrorCode::CompressionError,
                        reason: format!(
                            "header block exceeds max_header_block_bytes ({max_header_block})"
                        ),
                    });
                }
                let cont =
                    self.reader
                        .next()
                        .await?
                        .ok_or_else(|| H2Error::Connection {
                            code: ErrorCode::ProtocolError,
                            reason: "connection closed during CONTINUATION".into(),
                        })?;
                match cont {
                    Frame::Continuation {
                        stream_id,
                        end_headers,
                        fragment,
                    } if stream_id == h.stream_id => {
                        assembled.extend_from_slice(&fragment);
                        if end_headers {
                            break;
                        }
                    }
                    _ => {
                        return Err(H2Error::Connection {
                            code: ErrorCode::ProtocolError,
                            reason: "expected CONTINUATION frame".into(),
                        });
                    }
                }
            }
            Bytes::from(assembled)
        };

        let decoded = self
            .decoder
            .decode_header_block(&full_fragment)
            .map_err(H2Error::Hpack)?;

        let stream_id = h.stream_id;
        let actor = match self.streams.get_mut(&stream_id) {
            Some(a) => a,
            None => return Ok(()), // Unknown stream, ignore.
        };

        if !actor.got_headers {
            if let Err(e) = actor.state.transition(StreamEvent::RecvHeaders {
                end_stream: h.end_stream,
            }) {
                let err = map_state_err(stream_id, e);
                self.fail_stream(stream_id, err);
                return Ok(());
            }
            for header in &decoded {
                if header.name == ":status" {
                    actor.status = header
                        .value
                        .parse()
                        .map_err(|_| H2Error::Hpack("invalid :status".into()))?;
                } else if !header.name.starts_with(':') {
                    actor
                        .resp_headers
                        .push((header.name.clone(), header.value.clone()));
                }
            }
            actor.got_headers = true;
            if matches!(actor.status, 100..=199 | 204 | 304) {
                actor.drop_body = true;
            }
            // Streaming-response sinks deliver headers immediately.
            if matches!(
                actor.response_tx,
                Some(ResponseSink::StreamingEx { .. })
            ) {
                actor.deliver_headers_streaming();
            }
            if h.end_stream {
                self.complete_stream(stream_id);
            }
        } else {
            // Trailers.
            if let Err(e) = actor.state.transition(StreamEvent::RecvTrailers) {
                let err = map_state_err(stream_id, e);
                self.fail_stream(stream_id, err);
                return Ok(());
            }
            let mut th = Vec::new();
            for header in &decoded {
                th.push((header.name.clone(), header.value.clone()));
            }
            actor.trailers = Some(th);
            self.complete_stream(stream_id);
        }
        Ok(())
    }

    async fn on_data(&mut self, d: DataFrame) -> Result<(), H2Error> {
        let stream_id = d.stream_id;
        let len = d.data.len() as i64;

        // State machine + buffer.
        let complete;
        {
            let actor = match self.streams.get_mut(&stream_id) {
                Some(a) => a,
                None => {
                    // Unknown stream; still count against flow control.
                    self.conn_recv_window -= len;
                    self.maybe_top_up_conn_window().await?;
                    return Ok(());
                }
            };
            if let Err(e) = actor.state.transition(StreamEvent::RecvData {
                end_stream: d.end_stream,
            }) {
                let err = map_state_err(stream_id, e);
                self.fail_stream(stream_id, err);
                return Ok(());
            }
            if !actor.drop_body {
                let is_streaming = matches!(
                    actor.response_tx,
                    Some(ResponseSink::StreamingEx { .. })
                );
                if is_streaming {
                    // Forward the chunk to the consumer via the body
                    // channel. `try_send` so we never block the driver;
                    // backpressure is enforced through the bounded
                    // channel capacity — if the consumer is slow and
                    // the channel is full, we RST_STREAM with CANCEL.
                    if !d.data.is_empty() {
                        if let Some(ResponseSink::StreamingEx { body_tx, .. }) =
                            actor.response_tx.as_ref()
                        {
                            match body_tx.try_send(Ok(d.data.clone())) {
                                Ok(()) => {}
                                Err(mpsc::error::TrySendError::Full(_)) => {
                                    // Consumer too slow. For MVP we
                                    // block-forward by taking the
                                    // permit in a short await. This is
                                    // the simplest correct behaviour.
                                    let permit = body_tx.clone();
                                    if permit.send(Ok(d.data.clone())).await.is_err() {
                                        let err = H2Error::Stream {
                                            stream_id,
                                            code: ErrorCode::Cancel,
                                        };
                                        let _ = self
                                            .writer
                                            .write_rst_stream(stream_id, ErrorCode::Cancel)
                                            .await;
                                        self.fail_stream(stream_id, err);
                                        return Ok(());
                                    }
                                }
                                Err(mpsc::error::TrySendError::Closed(_)) => {
                                    let err = H2Error::Stream {
                                        stream_id,
                                        code: ErrorCode::Cancel,
                                    };
                                    let _ = self
                                        .writer
                                        .write_rst_stream(stream_id, ErrorCode::Cancel)
                                        .await;
                                    self.fail_stream(stream_id, err);
                                    return Ok(());
                                }
                            }
                        }
                    }
                } else {
                    let max_body = self.config.max_response_body_bytes;
                    if actor.body.len() + d.data.len() > max_body {
                        // Stream-level error, not connection-level.
                        let err = H2Error::Stream {
                            stream_id,
                            code: ErrorCode::Cancel,
                        };
                        let _ = self
                            .writer
                            .write_rst_stream(stream_id, ErrorCode::Cancel)
                            .await;
                        self.fail_stream(stream_id, err);
                        return Ok(());
                    }
                    actor.body.extend_from_slice(&d.data);
                }
            }
            actor.recv_window -= len;
            complete = d.end_stream;
        }

        self.conn_recv_window -= len;
        self.maybe_top_up_conn_window().await?;
        self.maybe_top_up_stream_window(stream_id).await?;

        if complete {
            self.complete_stream(stream_id);
        }
        Ok(())
    }

    async fn maybe_top_up_conn_window(&mut self) -> Result<(), H2Error> {
        let initial = self.config.initial_connection_window_size as i64;
        if self.conn_recv_window < initial / 2 {
            let increment = (initial - self.conn_recv_window)
                .max(1)
                .min(0x7FFF_FFFF) as u32;
            self.writer
                .write_window_update(&WindowUpdateFrame {
                    stream_id: 0,
                    increment,
                })
                .await?;
            self.conn_recv_window += increment as i64;
        }
        Ok(())
    }

    async fn maybe_top_up_stream_window(&mut self, stream_id: u32) -> Result<(), H2Error> {
        let initial = self.peer_settings.initial_window_size as i64;
        let needs_update = self
            .streams
            .get(&stream_id)
            .map(|a| a.recv_window < initial / 2)
            .unwrap_or(false);
        if needs_update {
            let current = self.streams.get(&stream_id).map(|a| a.recv_window).unwrap_or(0);
            let increment = (initial - current).max(1).min(0x7FFF_FFFF) as u32;
            self.writer
                .write_window_update(&WindowUpdateFrame {
                    stream_id,
                    increment,
                })
                .await?;
            if let Some(actor) = self.streams.get_mut(&stream_id) {
                actor.recv_window += increment as i64;
            }
        }
        Ok(())
    }

    fn complete_stream(&mut self, stream_id: u32) {
        if let Some(mut actor) = self.streams.remove(&stream_id) {
            actor.deliver_ok();
        }
        // Clean up pending queue.
        self.buffered_pending.retain(|&s| s != stream_id);
    }

    fn fail_stream(&mut self, stream_id: u32, err: H2Error) {
        if let Some(mut actor) = self.streams.remove(&stream_id) {
            actor.deliver_err(err);
        }
        self.buffered_pending.retain(|&s| s != stream_id);
    }

    fn active_stream_count(&self) -> u32 {
        self.streams
            .values()
            .filter(|a| {
                matches!(
                    a.state,
                    StreamState::Open
                        | StreamState::HalfClosedLocal
                        | StreamState::HalfClosedRemote
                )
            })
            .count() as u32
    }

    async fn graceful_shutdown(&mut self) -> Result<(), H2Error> {
        self.shutdown_started = true;
        // Send GOAWAY(NO_ERROR, last_stream_id = next_stream_id - 2).
        let last = self.next_stream_id.saturating_sub(2);
        let _ = self
            .writer
            .write_goaway(last, ErrorCode::NoError)
            .await;
        let _ = self.writer.flush().await;

        if self.streams.is_empty() {
            return Ok(());
        }

        // Drain — wait for in-flight responses up to a small timeout.
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            if self.streams.is_empty() {
                return Ok(());
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(());
            }
            let timeout = deadline - now;
            tokio::select! {
                biased;
                frame = self.reader.next() => {
                    match frame {
                        Ok(Some(f)) => self.on_inbound_frame(f).await?,
                        Ok(None) => return Ok(()),
                        Err(e) => return Err(e),
                    }
                    self.try_drain_pending().await?;
                }
                _ = tokio::time::sleep(timeout) => {
                    return Ok(());
                }
            }
        }
    }
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

fn map_state_err(stream_id: u32, e: StreamStateError) -> H2Error {
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

fn clone_err(e: &H2Error) -> H2Error {
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
