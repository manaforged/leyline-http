//! HTTP/1.1 keep-alive pool entry point.
//!
//! Owns the request-serialisation + response-parsing logic shared
//! between fresh and reused connections. Each successful
//! request/response exchange ends with a reusability check; reusable
//! streams are parked back in the pool, everything else is dropped so
//! the next request starts clean.

use std::borrow::Cow;
use std::io;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::StreamExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{OwnedSemaphorePermit, mpsc};

use crate::core::BodyStream;
use crate::tls::{FingerprintConnector, TlsError};

use crate::pool::types::PoolKey;
use crate::pool::types::Transport;
use crate::pool::{H1Slot, Pool, TlsInfo, make_key};

/// Maximum request-line + headers size. Matches the core transport
/// limit that pre-dated the pool refactor.
pub const MAX_H1_HEADER_BYTES: usize = 64 * 1024;

/// Maximum body size the H1 pool will accept or send. 100 MiB is the
/// same cap the core transport enforced before the move.
pub const MAX_H1_BODY_BYTES: usize = 100 * 1024 * 1024;

/// Marker trait for the two concrete I/O types we hold in the pool:
/// `TlsStream` over TCP for `https://` and bare `TcpStream` for
/// `http://`. Kept as a trait object so [`Pool`] can service both
/// schemes behind a single key.
pub trait H1Io: AsyncRead + AsyncWrite + Send + Unpin + 'static {}
impl<T> H1Io for T where T: AsyncRead + AsyncWrite + Send + Unpin + 'static {}

/// Request-target style: origin-form `/path?q=1` for direct
/// connections, absolute-form `http://host/path?q=1` for plaintext
/// HTTP proxies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum H1Target {
    /// `GET /path?q=1 HTTP/1.1`
    OriginForm,
    /// `GET http://host/path?q=1 HTTP/1.1`
    AbsoluteForm,
}

/// Request body shape accepted by [`send_request_h1_pooled`].
///
/// Mirrors the core `Body` enum without depending on it, so the pool
/// stays agnostic of the caller's body type.
pub enum H1Body {
    /// No body.
    Empty,
    /// A fully-materialised byte buffer. `Content-Length` is set
    /// automatically when absent.
    Buffered(Bytes),
    /// A streaming body with a known exact content length.
    /// `Content-Length` is set automatically when absent.
    FixedStream {
        /// The stream of body chunks.
        stream: Pin<Box<dyn futures_util::Stream<Item = std::io::Result<Bytes>> + Send + 'static>>,
        /// Exact body length in bytes.
        length: u64,
    },
    /// A streaming body with unknown length. Framed as
    /// `Transfer-Encoding: chunked` when absent.
    ChunkedStream {
        /// The stream of body chunks.
        stream: Pin<Box<dyn futures_util::Stream<Item = std::io::Result<Bytes>> + Send + 'static>>,
    },
}

impl H1Body {
    fn is_stream(&self) -> bool {
        matches!(
            self,
            H1Body::FixedStream { .. } | H1Body::ChunkedStream { .. }
        )
    }
}

/// Response body produced by the H1 pool.
pub enum H1ResponseBody {
    /// The fully-drained response body.
    Buffered(Vec<u8>),
    /// An incrementally-streamed body. A background pump task owns the
    /// connection, forwards framed chunks to the consumer, and reinstates
    /// the connection to the pool on a clean full drain (or drops it).
    Streaming(BodyStream),
}

/// Response returned by [`send_request_h1_pooled`].
pub struct H1Response {
    /// HTTP status code.
    pub status: u16,
    /// Response headers in wire order.
    pub headers: Vec<(String, String)>,
    /// Response body. H1 streaming responses are a future extension
    /// — today every response is fully drained so the connection is
    /// either reusable or dropped before this function returns.
    pub body: H1ResponseBody,
    /// TLS handshake snapshot for the connection that served the
    /// request, or `None` for plaintext HTTP.
    pub tls: Option<TlsInfo>,
}

/// Errors surfaced by [`send_request_h1_pooled`].
#[derive(Debug, thiserror::Error)]
pub enum H1PooledError {
    /// Caller misconfiguration (bad URL, unsupported scheme, …).
    #[error("{0}")]
    Config(String),
    /// TLS handshake failure.
    #[error(transparent)]
    Tls(#[from] TlsError),
    /// Plain I/O error during connect, send, or receive.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Protocol-level error parsing a response (bad status line,
    /// oversize headers, malformed chunked body, …). NOT retryable — the
    /// message can interpolate attacker-controlled header bytes, so it must
    /// never feed a retry decision.
    #[error("http: {0}")]
    Http(String),
    /// The connection closed before the response was fully received — a
    /// transport-level EOF mid-exchange, distinct from a framing error.
    /// Safe to retry on a fresh connection (mapped to `Error::Io`
    /// `UnexpectedEof` at the core boundary).
    #[error("connection closed: {0}")]
    ConnectionClosed(String),
}

// thiserror brings the Display/Error impls; nothing else needed.

/// Non-blocking liveness probe for a pooled keep-alive socket.
///
/// `checkout_h1` only sees application-level state (the idle deque) — it
/// cannot tell that a peer closed its half of the connection while it sat
/// idle. A keep-alive server with a shorter idle timeout than our pool (a
/// Node default is 5s vs our 300s) reaps the socket from under us; the next
/// write then fails mid-request. This poll catches that before we commit a
/// request to the dead socket.
///
/// Returns `false` when the socket has hit EOF (read-ready, zero bytes), has
/// errored, or already has bytes waiting before we sent anything (a framing
/// desync we must not reuse). Returns `true` only for the normal idle
/// keep-alive state: open, with no data pending.
fn conn_is_live(io: &mut dyn H1Io) -> bool {
    use std::task::{Context, Poll};
    use tokio::io::ReadBuf;

    let mut probe = [0u8; 1];
    let mut buf = ReadBuf::new(&mut probe);
    let mut cx = Context::from_waker(std::task::Waker::noop());
    match Pin::new(io).poll_read(&mut cx, &mut buf) {
        // No data pending and not closed — the expected idle-keep-alive state.
        Poll::Pending => true,
        // Ready with zero bytes is EOF; ready with bytes is a pre-request
        // desync. Either way the connection is not safe to reuse.
        Poll::Ready(Ok(())) => false,
        Poll::Ready(Err(_)) => false,
    }
}

/// Check out a pooled H1 connection that is still alive at the socket level.
///
/// Drains and discards any pooled entries that already hit EOF / error so a
/// stale keep-alive connection becomes a clean cache miss (fresh connect)
/// instead of a failed request, draining until a live connection is found or
/// the destination's deque is empty.
fn checkout_live_h1(pool: &Arc<Pool>, key: &PoolKey) -> Option<(H1Slot, TlsInfo)> {
    while let Some((mut slot, tls)) = pool.checkout_h1(key) {
        if conn_is_live(slot.io.as_mut()) {
            return Some((slot, tls));
        }
        // Dead pooled socket caught before use — count it as a probe catch
        // (distinct from a mid-exchange failure) and drop it (the checked-out
        // slot is already removed from the deque); loop to the next warm entry.
        pool.note_h1_stale_probed();
    }
    None
}

/// Send an HTTP/1.1 request over a pooled connection, opening a
/// fresh TCP + TLS handshake on miss.
///
/// On a cache hit the owned stream is taken out of the pool via
/// `Option::take()` (single-checkout semantics — H1 cannot multiplex).
/// After the response body is fully drained, the stream is reinstated
/// under the same key when it's still reusable; otherwise it is
/// dropped.
///
/// On any I/O error mid-exchange the stream is dropped and the entry
/// is invalidated so the next request opens a fresh connection.
#[tracing::instrument(
    name = "pool.send_request_h1",
    level = "debug",
    skip_all,
    fields(
        http.method = method,
        http.host = host,
        http.port = port,
        proxied = proxy.is_some(),
        pool.hit = tracing::field::Empty,
    )
)]
#[expect(
    clippy::too_many_arguments,
    reason = "flat per-request wire fields across one internal call path"
)]
pub async fn send_request_h1_pooled(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    scheme: &str,
    host: &str,
    port: u16,
    method: &str,
    url: &url::Url,
    headers: Vec<(String, String)>,
    body: H1Body,
    proxy: Option<&str>,
    target: H1Target,
    stream: bool,
) -> Result<H1Response, H1PooledError> {
    // Wire-shape validation happens BEFORE any TCP connect so an
    // attacker-controlled header / method / URL never causes a real
    // network side effect. CWE-93 request-smuggling defence.
    if !is_valid_token(method) {
        return Err(H1PooledError::Config(format!(
            "invalid HTTP method `{method}`: non-token bytes not allowed"
        )));
    }
    for (name, value) in &headers {
        if !is_valid_token(name) {
            return Err(H1PooledError::Config(format!(
                "invalid header name `{name}`: non-token bytes not allowed"
            )));
        }
        if !is_valid_header_value(value) {
            return Err(H1PooledError::Config(format!(
                "invalid value for header `{name}`: control characters not allowed"
            )));
        }
    }

    pool.evict_idle();

    let key = make_key(scheme, host, port, proxy, Transport::Tcp);

    // Acquire one of this destination's H1 connection permits before any pool
    // work. This caps concurrent H1 connections per host at
    // `max_h1_conns_per_host` (default 6 — a browser's per-host socket limit)
    // and queues the overflow here instead of opening unbounded sockets. Held
    // for the whole exchange: dropping `_permit` at function exit frees the
    // slot — and the warm connection we return to the pool just below — for a
    // queued request, so the next waiter reuses it rather than handshaking.
    let _permit = pool.acquire_h1_permit(&key).await;

    // Streaming responses hand the connection to a background pump that owns
    // it for the body's lifetime — route there before the buffered retry
    // setup, moving the permit and key into the pump.
    if stream {
        return send_request_h1_streaming(
            pool, connector, scheme, host, port, method, url, headers, body, proxy, target,
            _permit, key,
        )
        .await;
    }

    // Replay discipline: a buffered body may be re-sent on a fresh
    // connection only when the method is idempotent — a pooled connection
    // that dies mid-exchange may have already received and processed the
    // request. Streaming bodies are one-shot and never replay.
    let body_is_stream = body.is_stream();
    let replayable = crate::core::retry::is_idempotent(method);
    let retry_buf: Option<Bytes> = match &body {
        H1Body::Buffered(b) => Some(b.clone()),
        _ => None,
    };
    let mut body = body;

    // Try pooled connection first. Probe each candidate for socket-level
    // liveness so a keep-alive peer that closed under us becomes a clean miss
    // rather than a failed write.
    if let Some((slot, tls)) = checkout_live_h1(pool, &key) {
        let pooled_body = std::mem::replace(&mut body, H1Body::Empty);
        let mut io = slot.io;
        match exchange_on_stream(
            io.as_mut(),
            method,
            url,
            headers.clone(),
            pooled_body,
            target,
        )
        .await
        {
            Ok((resp, reusable)) => {
                tracing::Span::current().record("pool.hit", true);
                if reusable {
                    pool.return_h1(key.clone(), H1Slot { io }, tls.clone());
                }
                // else: drop `io` (deliberately non-reusable). We touch only
                // the connection we checked out — sibling warm connections to
                // this host stay pooled.
                return Ok(H1Response {
                    status: resp.status,
                    headers: resp.headers,
                    body: H1ResponseBody::Buffered(resp.body),
                    tls: tls_for_scheme(scheme, &tls),
                });
            }
            Err(e) => {
                tracing::info!(
                    target: "leyline::pool",
                    host = %key.host,
                    port = key.port,
                    proxied = key.proxy.is_some(),
                    error = %e,
                    "pool stale hit -- pooled h1 stream failed mid-request, opening fresh"
                );
                // The checked-out connection is already removed from the pool;
                // drop it and fall through to a fresh connection (still under
                // our permit). Sibling pooled connections to this host are left
                // intact — each is validated on its own checkout.
                pool.note_h1_dead();
                if body_is_stream {
                    return Err(e);
                }
                if !replayable {
                    return Err(e);
                }
                if let Some(buf) = &retry_buf {
                    body = H1Body::Buffered(buf.clone());
                }
            }
        }
    }
    tracing::Span::current().record("pool.hit", false);

    // Miss — fresh connection (still under our permit).
    let (io, tls): (Box<dyn H1Io>, TlsInfo) =
        open_new(connector, scheme, host, port, proxy).await?;

    let mut slot = H1Slot { io };
    let result = exchange_on_stream(slot.io.as_mut(), method, url, headers, body, target).await;

    match result {
        Ok((resp, reusable)) => {
            if reusable {
                pool.return_h1(key, slot, tls.clone());
                // Count the install only once the fresh connection is actually
                // pooled — a non-reusable completion (e.g. framing conflict) is
                // opened but never installed and must not be counted.
                pool.note_h1_install();
            }
            // If not reusable, drop the slot so it closes cleanly.
            Ok(H1Response {
                status: resp.status,
                headers: resp.headers,
                body: H1ResponseBody::Buffered(resp.body),
                tls: tls_for_scheme(scheme, &tls),
            })
        }
        Err(e) => Err(e),
    }
}

fn tls_for_scheme(scheme: &str, tls: &TlsInfo) -> Option<TlsInfo> {
    if scheme == "https" {
        Some(tls.clone())
    } else {
        None
    }
}

/// Open a fresh TCP (+ optional TLS) stream for the given destination
/// and return it as a boxed `H1Io` alongside the TLS snapshot.
async fn open_new(
    connector: &FingerprintConnector,
    scheme: &str,
    host: &str,
    port: u16,
    proxy: Option<&str>,
) -> Result<(Box<dyn H1Io>, TlsInfo), H1PooledError> {
    match scheme {
        "https" => {
            let tls_stream = connector.connect_h1(host, port, proxy).await?;
            let tls = TlsInfo {
                peer_cert_der: tls_stream.peer_cert_der.clone(),
                version: tls_stream.tls_version.clone(),
                cipher: tls_stream.tls_cipher.clone(),
            };
            let io: Box<dyn H1Io> = Box::new(tls_stream.stream);
            Ok((io, tls))
        }
        "http" => {
            let stream = if let Some(proxy_url) = proxy {
                let parsed = url::Url::parse(proxy_url)
                    .map_err(|e| H1PooledError::Config(format!("invalid proxy URL: {e}")))?;
                if parsed.scheme() != "http" {
                    return Err(H1PooledError::Config(
                        "plaintext HTTP currently supports http:// proxies only".into(),
                    ));
                }
                let proxy_host = parsed
                    .host_str()
                    .ok_or_else(|| H1PooledError::Config("proxy has no host".into()))?;
                // `port()` returns None for a scheme's default port, so
                // `http://host:80` would silently resolve to 8080. See
                // `leyline-tls::proxy::http::connect` for the same fix.
                let proxy_port = parsed.port_or_known_default().unwrap_or(8080);
                TcpStream::connect((proxy_host, proxy_port)).await?
            } else {
                TcpStream::connect((host, port)).await?
            };
            let io: Box<dyn H1Io> = Box::new(stream);
            Ok((io, TlsInfo::default()))
        }
        other => Err(H1PooledError::Config(format!(
            "unsupported URL scheme for HTTP/1.1: {other}"
        ))),
    }
}

/// Wire-level response carried back by the exchange helper.
struct WireResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// Bounded backpressure for the streaming pump: the pump blocks on `send`
/// when the consumer is behind, so a slow reader naturally rate-limits the
/// socket reads instead of buffering an unbounded body in memory.
const STREAM_CHANNEL_DEPTH: usize = 16;

/// How a response body is delimited on the wire.
#[derive(Debug, Clone, Copy)]
enum BodyFraming {
    /// No body (HEAD, 101, 204, 304).
    None,
    /// Exact length from `Content-Length`.
    Fixed(u64),
    /// `Transfer-Encoding: chunked`.
    Chunked,
    /// No framing — the body ends when the server closes the connection.
    ToClose,
}

/// Parsed response head plus the framing of the not-yet-read body and the
/// body bytes already buffered while reading the header block.
struct H1Head {
    status: u16,
    headers: Vec<(String, String)>,
    minor: u8,
    framing: BodyFraming,
    initial_body: Vec<u8>,
}

/// Owns the H1 connection for the lifetime of a streamed response body and
/// reinstates it to the pool after a clean full drain.
struct H1StreamPump {
    io: Box<dyn H1Io>,
    permit: OwnedSemaphorePermit,
    pool: Arc<Pool>,
    key: PoolKey,
    tls: TlsInfo,
    framing: BodyFraming,
    initial_body: Vec<u8>,
    reusable: bool,
    /// Count a pool install only for a fresh connection — a reused one was
    /// already counted when first installed.
    count_install: bool,
    tx: mpsc::Sender<io::Result<Bytes>>,
}

/// Streaming variant of [`send_request_h1_pooled`]: reads the response head,
/// then hands the connection to a background pump that forwards body chunks
/// and reinstates the connection on a clean full drain. The permit moves into
/// the pump and releases when streaming ends.
/// Parsed HTTP/1.x response head: status + headers + http/1.x minor
/// version (`1` for HTTP/1.1, `0` for HTTP/1.0).
type ParsedHead = (u16, Vec<(String, String)>, u8);

/// Parsed HTTP/1.x response: status + headers + body + http/1.x
/// minor version.
type ParsedResponse = (u16, Vec<(String, String)>, Vec<u8>, u8);

async fn read_chunk_trailers<S>(stream: &mut S, buf: &mut Vec<u8>) -> Result<(), H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    loop {
        let line_end = read_until_crlf(stream, buf).await?;
        let empty = line_end == 0;
        buf.drain(..line_end + 2);
        if empty {
            return Ok(());
        }
    }
}

/// Upper bound on a single CRLF-delimited line inside a chunked body
/// (size lines and trailers). `read_h1_headers` caps the header block;
/// without the same cap here, a hostile server streaming one endless
/// CRLF-less line grows the buffer without bound until the total timeout.
const MAX_H1_CHUNK_LINE_BYTES: usize = 16 * 1024;

async fn read_until_crlf<S>(stream: &mut S, buf: &mut Vec<u8>) -> Result<usize, H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    loop {
        if let Some(pos) = buf.windows(2).position(|w| w == b"\r\n") {
            return Ok(pos);
        }
        if buf.len() > MAX_H1_CHUNK_LINE_BYTES {
            return Err(H1PooledError::Http(format!(
                "chunked size/trailer line exceeds {MAX_H1_CHUNK_LINE_BYTES} bytes"
            )));
        }
        read_more(stream, buf).await?;
    }
}

async fn read_until_available<S>(
    stream: &mut S,
    buf: &mut Vec<u8>,
    len: usize,
) -> Result<(), H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    while buf.len() < len {
        read_more(stream, buf).await?;
    }
    Ok(())
}

async fn read_more<S>(stream: &mut S, buf: &mut Vec<u8>) -> Result<(), H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    let mut tmp = [0u8; 8192];
    let n = stream.read(&mut tmp).await?;
    if n == 0 {
        return Err(H1PooledError::ConnectionClosed(
            "during chunked body".into(),
        ));
    }
    buf.extend_from_slice(&tmp[..n]);
    if buf.len() > MAX_H1_BODY_BYTES {
        return Err(H1PooledError::Http(format!(
            "HTTP/1.1 body exceeds {MAX_H1_BODY_BYTES} bytes"
        )));
    }
    Ok(())
}

fn path_and_query(url: &url::Url) -> String {
    let path = if url.path().is_empty() {
        "/"
    } else {
        url.path()
    };
    match url.query() {
        Some(query) => format!("{path}?{query}"),
        None => path.to_string(),
    }
}

fn authority_for(url: &url::Url, host: &str, port: u16) -> String {
    let is_default_port =
        (url.scheme() == "https" && port == 443) || (url.scheme() == "http" && port == 80);
    if is_default_port {
        host.to_string()
    } else {
        format!("{host}:{port}")
    }
}

fn contains_header(headers: &[(String, String)], name: &str) -> bool {
    headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
}

/// Predicate: `s` is a non-empty sequence of RFC 9110 `tchar`s — the
/// byte set permitted for HTTP method names and header names. Used to
/// reject header-injection payloads at the wire boundary (CWE-93).
fn is_valid_token(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    s.bytes().all(|b| {
        // RFC 9110 §5.6.2 token = 1*tchar.
        // tchar = "!" / "#" / "$" / "%" / "&" / "'" / "*" / "+" /
        //         "-" / "." / "^" / "_" / "`" / "|" / "~" / DIGIT / ALPHA
        matches!(
            b,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
                | b'0'..=b'9'
                | b'A'..=b'Z'
                | b'a'..=b'z'
        )
    })
}

/// Predicate: `s` is a valid HTTP header field-value.
///
/// RFC 9110 §5.5: `field-value = *( field-content / obs-text )`,
/// where `field-content = field-vchar [ 1*( SP / HTAB / field-vchar )
/// field-vchar ]` and `field-vchar = VCHAR / obs-text`. We permit
/// visible ASCII, SP, HTAB, and 0x80..=0xFF (obs-text for UTF-8 etc.)
/// but reject CR / LF / NUL and the remaining control characters —
/// the exact bytes an attacker needs for header-splitting.
fn is_valid_header_value(s: &str) -> bool {
    s.bytes()
        .all(|b| matches!(b, b'\t' | b' '..=b'~' | 0x80..=0xFF))
}

/// Predicate: `s` is a valid HTTP request-target.
///
/// URL parsing blocks raw CR/LF in the authority, but origin-form
/// and absolute-form targets are serialised via the percent-encoded
/// path + query which a malicious-but-accepted URL could in
/// principle still smuggle through. Defence-in-depth: the request
/// line is `METHOD SP TARGET SP HTTP/1.1 CRLF`, so any CR/LF/SP/NUL
/// inside `TARGET` splits it.
fn is_valid_request_target(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    s.bytes().all(|b| b > 0x20 && b != 0x7F)
}

fn h1_header_name(name: &str) -> Cow<'_, str> {
    match name {
        _ if name.eq_ignore_ascii_case("host") => Cow::Borrowed("Host"),
        _ if name.eq_ignore_ascii_case("connection") => Cow::Borrowed("Connection"),
        _ if name.eq_ignore_ascii_case("user-agent") => Cow::Borrowed("User-Agent"),
        _ if name.eq_ignore_ascii_case("accept") => Cow::Borrowed("Accept"),
        _ if name.eq_ignore_ascii_case("accept-encoding") => Cow::Borrowed("Accept-Encoding"),
        _ if name.eq_ignore_ascii_case("accept-language") => Cow::Borrowed("Accept-Language"),
        _ if name.eq_ignore_ascii_case("content-length") => Cow::Borrowed("Content-Length"),
        _ if name.eq_ignore_ascii_case("content-type") => Cow::Borrowed("Content-Type"),
        _ if name.eq_ignore_ascii_case("cookie") => Cow::Borrowed("Cookie"),
        _ if name.eq_ignore_ascii_case("authorization") => Cow::Borrowed("Authorization"),
        _ if name.eq_ignore_ascii_case("proxy-authorization") => {
            Cow::Borrowed("Proxy-Authorization")
        }
        _ if name.eq_ignore_ascii_case("origin") => Cow::Borrowed("Origin"),
        _ if name.eq_ignore_ascii_case("referer") => Cow::Borrowed("Referer"),
        _ if name.eq_ignore_ascii_case("upgrade") => Cow::Borrowed("Upgrade"),
        _ => Cow::Borrowed(name),
    }
}

fn header_first<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn header_contains_token(headers: &[(String, String)], name: &str, token: &str) -> bool {
    header_first(headers, name).is_some_and(|v| {
        v.split(',')
            .any(|part| part.trim().eq_ignore_ascii_case(token))
    })
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

mod parse;
mod streaming;
mod wire;
use parse::*;
use streaming::*;
use wire::*;

#[cfg(test)]
mod tests;
