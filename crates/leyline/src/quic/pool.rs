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
//! Response bodies are buffered and delivered on stream completion (matching
//! today's H3 transport contract). Incremental H3 response streaming is a
//! separate, deferred piece of work.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use leyline_quiche as quiche;
use quiche::h3::NameValue;
use tokio::sync::{mpsc, oneshot};

use crate::pool::TlsInfo;
use crate::profile::BrowserProfile;
use crate::quic::config::H3Config;
use crate::quic::connection::{
    check_body_budget, close_reason, connect_and_handshake, flush_egress, EstablishedH3, H3Response,
};

/// Max number of outstanding request commands the driver buffers before the
/// handle's `send` applies back-pressure. Generous; real workloads rarely
/// have more than a few thousand concurrent requests to one host.
const COMMAND_CHANNEL_CAPACITY: usize = 1024;

/// A request fanned from an [`H3Client`] handle to the driver.
enum H3Command {
    Request {
        /// Pre-built HTTP/3 header list (pseudo-headers first), owned so it
        /// crosses the channel without borrowing the caller.
        headers: Vec<quiche::h3::Header>,
        body: Option<Bytes>,
        resp_tx: oneshot::Sender<Result<H3Response, String>>,
    },
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
    /// Send a request over a multiplexed stream and await the buffered
    /// response. Concurrent calls run on independent streams.
    pub async fn send_request(
        &self,
        method: &str,
        authority: &str,
        path: &str,
        headers: &[(String, String)],
        body: Option<Bytes>,
    ) -> Result<H3Response, H3SendError> {
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

        let (resp_tx, resp_rx) = oneshot::channel();
        // tx.send failing means the driver is gone and the command never
        // entered the queue — the request provably never went out.
        self.tx
            .send(H3Command::Request {
                headers: h3_headers,
                body,
                resp_tx,
            })
            .await
            .map_err(|_| H3SendError::NotSent("h3 driver task has exited".into()))?;

        // Past this point the driver owns the request; any failure is
        // ambiguous (it may have hit the wire), so it is not replay-safe.
        match resp_rx.await {
            Ok(Ok(resp)) => Ok(resp),
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
    /// Outbound request body not yet fully written (flow-control parked).
    pending_body: Bytes,
    body_offset: usize,
}

impl H3Stream {
    fn new(resp_tx: oneshot::Sender<Result<H3Response, String>>, body: Option<Bytes>) -> Self {
        let pending_body = match body {
            Some(b) if !b.is_empty() => b,
            _ => Bytes::new(),
        };
        Self {
            resp_tx: Some(resp_tx),
            status: 0,
            headers: Vec::new(),
            body: Vec::new(),
            pending_body,
            body_offset: 0,
        }
    }

    fn body_remaining(&self) -> bool {
        self.body_offset < self.pending_body.len()
    }

    fn deliver(&mut self, result: Result<H3Response, String>) {
        if let Some(tx) = self.resp_tx.take() {
            let _ = tx.send(result);
        }
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
    let closed = Arc::new(AtomicBool::new(false));

    let driver = H3Driver {
        established,
        command_rx,
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
        let closed = self.closed;
        let mut streams = self.streams;

        let mut out = vec![0u8; max_udp_payload];
        let mut buf = vec![0u8; 65_535];
        let mut pending: VecDeque<H3Command> = VecDeque::new();
        let mut commands_closed = false;

        loop {
            // Start any queued requests now that the connection can take them,
            // then (re)attempt any flow-control-parked request bodies.
            start_pending(&mut h3, &mut conn, &mut streams, &mut pending);
            retry_request_bodies(&mut h3, &mut conn, &mut streams);

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

            let timeout = conn.timeout().unwrap_or(Duration::from_secs(5));

            tokio::select! {
                cmd = command_rx.recv(), if !commands_closed => match cmd {
                    Some(cmd) => pending.push_back(cmd),
                    None => commands_closed = true,
                },
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
) {
    while let Some(H3Command::Request { headers, body, .. }) = pending.front() {
        let fin = body.as_ref().is_none_or(Bytes::is_empty);
        match h3.send_request(conn, headers, fin) {
            Ok(stream_id) => {
                let Some(H3Command::Request { body, resp_tx, .. }) = pending.pop_front() else {
                    unreachable!("front matched Request above");
                };
                let mut stream = H3Stream::new(resp_tx, body);
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

/// Re-attempt any request bodies that flow control parked mid-write.
fn retry_request_bodies(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    streams: &mut HashMap<u64, H3Stream>,
) {
    for (stream_id, stream) in streams.iter_mut() {
        if stream.body_remaining() {
            write_request_body(h3, conn, *stream_id, stream);
        }
    }
}

/// Write as much of a stream's pending request body as flow control allows.
/// `fin` rides the final bytes, so quiche marks the stream finished only once
/// the whole body is flushed.
fn write_request_body(
    h3: &mut quiche::h3::Connection,
    conn: &mut quiche::Connection,
    stream_id: u64,
    stream: &mut H3Stream,
) {
    while stream.body_offset < stream.pending_body.len() {
        let chunk = &stream.pending_body[stream.body_offset..];
        match h3.send_body(conn, stream_id, chunk, true) {
            Ok(0) => break,
            Ok(written) => stream.body_offset += written,
            Err(quiche::h3::Error::Done) | Err(quiche::h3::Error::StreamBlocked) => break,
            Err(e) => {
                stream.deliver(Err(format!("h3 send_body: {e}")));
                // Leave the dead stream in the map; the peer Reset / connection
                // teardown removes it. Stop trying to write it.
                stream.pending_body = Bytes::new();
                stream.body_offset = 0;
                break;
            }
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
                        check_body_budget(stream.body.len(), n, max_response_body_bytes)
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
                    stream.body.extend_from_slice(&scratch[..n]);
                }
            }
            Ok((stream_id, quiche::h3::Event::Finished)) => {
                if let Some(mut stream) = streams.remove(&stream_id) {
                    // The peer finished responding. If we hadn't finished
                    // uploading the request body (an early 4xx/413), abort our
                    // send side — otherwise the half-open stream lingers and
                    // leaks QUIC stream credit until the connection runs out.
                    if stream.body_remaining() {
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
            Ok((stream_id, quiche::h3::Event::Reset(e))) => {
                if let Some(mut stream) = streams.remove(&stream_id) {
                    stream.deliver(Err(format!("h3 stream reset: {e}")));
                }
            }
            Ok((_, quiche::h3::Event::GoAway)) | Ok((_, quiche::h3::Event::PriorityUpdate)) => {}
            Err(quiche::h3::Error::Done) => return Ok(()),
            Err(e) => return Err(format!("h3 poll: {e}")),
        }
    }
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
        stream.deliver(Err(reason.clone()));
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
        assert!(!H3Stream::new(tx, None).body_remaining());
        let (tx, _rx) = oneshot::channel();
        assert!(
            !H3Stream::new(tx, Some(Bytes::new())).body_remaining(),
            "an empty Bytes body must not park as pending"
        );
        let (tx, _rx) = oneshot::channel();
        assert!(H3Stream::new(tx, Some(Bytes::from_static(b"x"))).body_remaining());
    }

    #[tokio::test]
    async fn deliver_is_once_only() {
        let (tx, rx) = oneshot::channel();
        let mut stream = H3Stream::new(tx, None);
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
        streams.insert(1u64, H3Stream::new(tx, None));

        let mut pending = VecDeque::new();
        let (tx2, rx_pending) = oneshot::channel();
        pending.push_back(H3Command::Request {
            headers: Vec::new(),
            body: None,
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
