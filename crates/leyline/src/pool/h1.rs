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
use crate::tls::ConnectorVariant;

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
    #[error("tls: {0}")]
    Tls(String),
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
#[allow(clippy::too_many_arguments)]
pub async fn send_request_h1_pooled(
    pool: &Arc<Pool>,
    connector: &ConnectorVariant,
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

    let key = make_key(host, port, proxy, Transport::Tcp);

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

    // Streaming bodies are one-shot — no retry possible. For
    // buffered bodies we keep a clone in case the pooled attempt
    // fails before any bytes reach the server and we need a fresh
    // connection.
    let body_is_stream = body.is_stream();
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
    connector: &ConnectorVariant,
    scheme: &str,
    host: &str,
    port: u16,
    proxy: Option<&str>,
) -> Result<(Box<dyn H1Io>, TlsInfo), H1PooledError> {
    match scheme {
        "https" => {
            let tls_stream = connector
                .connect_h1(host, port, proxy)
                .await
                .map_err(|e| H1PooledError::Tls(e.to_string()))?;
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

/// Serialise and send an HTTP/1.1 request head + body on `stream`.
/// Returns whether the caller asked to close the connection
/// (`Connection: close`), which feeds the post-response reuse decision.
/// Shared by the buffered exchange and the streaming head exchange.
async fn send_h1_request(
    stream: &mut dyn H1Io,
    method: &str,
    url: &url::Url,
    mut headers: Vec<(String, String)>,
    body: H1Body,
    target: H1Target,
) -> Result<bool, H1PooledError> {
    // ─── Wire-shape validation (CWE-93, request smuggling) ──────────
    //
    // Every caller-supplied byte that lands on the keep-alive socket
    // must be rejected for CR/LF/NUL before we serialise it. A
    // single `\r\n` in a header value or method splits the request
    // and lets an attacker smuggle a second request into the reused
    // pool connection.
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
    let host = url
        .host_str()
        .ok_or_else(|| H1PooledError::Config("no host in URL".into()))?;
    let port = url.port_or_known_default().ok_or_else(|| {
        H1PooledError::Config(format!("no default port for scheme {}", url.scheme()))
    })?;
    let authority = authority_for(url, host, port);
    let path = path_and_query(url);
    let request_target = match target {
        H1Target::OriginForm => path,
        H1Target::AbsoluteForm => url.as_str().to_string(),
    };
    // URL parsing rejects CR/LF in host, but the path / query can
    // contain percent-encoded bytes. Validate the final target
    // string as a defence-in-depth: no raw CR/LF/SP/NUL/HTAB. The
    // `request_target` is what goes on the request-line, so any
    // control char here is a smuggling vector.
    if !is_valid_request_target(&request_target) {
        return Err(H1PooledError::Config(format!(
            "invalid request target `{request_target}`: control characters not allowed"
        )));
    }

    if !contains_header(&headers, "host") {
        headers.insert(0, ("Host".into(), authority));
    }

    let has_cl = contains_header(&headers, "content-length");
    let has_te = contains_header(&headers, "transfer-encoding");

    enum Framing {
        None,
        Buffered(Bytes),
        FixedStream {
            stream:
                Pin<Box<dyn futures_util::Stream<Item = std::io::Result<Bytes>> + Send + 'static>>,
            length: u64,
        },
        ChunkedStream {
            stream:
                Pin<Box<dyn futures_util::Stream<Item = std::io::Result<Bytes>> + Send + 'static>>,
        },
    }

    let framing = match body {
        H1Body::Empty => {
            if method_typically_has_body(method) && !has_cl && !has_te {
                headers.push(("Content-Length".into(), "0".into()));
            }
            Framing::None
        }
        H1Body::Buffered(b) => {
            if !has_cl && !has_te {
                headers.push(("Content-Length".into(), b.len().to_string()));
            }
            Framing::Buffered(b)
        }
        H1Body::FixedStream { stream, length } => {
            if !has_cl && !has_te {
                headers.push(("Content-Length".into(), length.to_string()));
            }
            Framing::FixedStream { stream, length }
        }
        H1Body::ChunkedStream { stream } => {
            if !has_te {
                headers.push(("Transfer-Encoding".into(), "chunked".into()));
            }
            Framing::ChunkedStream { stream }
        }
    };

    if !contains_header(&headers, "connection") {
        headers.push(("Connection".into(), "keep-alive".into()));
    }

    // Serialise + send the request head.
    let mut req = Vec::new();
    req.extend_from_slice(format!("{method} {request_target} HTTP/1.1\r\n").as_bytes());
    for (name, value) in &headers {
        let name = h1_header_name(name);
        req.extend_from_slice(name.as_bytes());
        req.extend_from_slice(b": ");
        req.extend_from_slice(value.as_bytes());
        req.extend_from_slice(b"\r\n");
    }
    req.extend_from_slice(b"\r\n");
    stream.write_all(&req).await?;

    match framing {
        Framing::None => {}
        Framing::Buffered(b) => {
            stream.write_all(&b).await?;
        }
        Framing::FixedStream {
            stream: mut body_stream,
            length,
        } => {
            let mut sent: u64 = 0;
            while let Some(chunk) = body_stream.next().await {
                let chunk: Bytes = chunk?;
                if sent + chunk.len() as u64 > length {
                    return Err(H1PooledError::Http(
                        "streaming body exceeded declared content-length".into(),
                    ));
                }
                stream.write_all(&chunk).await?;
                sent += chunk.len() as u64;
            }
            if sent != length {
                return Err(H1PooledError::Http(format!(
                    "streaming body ended before declared content-length ({sent}/{length})"
                )));
            }
        }
        Framing::ChunkedStream {
            stream: mut body_stream,
        } => {
            while let Some(chunk) = body_stream.next().await {
                let chunk: Bytes = chunk?;
                if chunk.is_empty() {
                    continue;
                }
                let hdr = format!("{:X}\r\n", chunk.len());
                stream.write_all(hdr.as_bytes()).await?;
                stream.write_all(&chunk).await?;
                stream.write_all(b"\r\n").await?;
            }
            stream.write_all(b"0\r\n\r\n").await?;
        }
    }

    stream.flush().await?;

    // Whether the caller explicitly asked us to close the connection via
    // `Connection: close` — feeds the post-response reuse decision.
    Ok(header_contains_token(&headers, "connection", "close"))
}

/// Decide whether a keep-alive connection may be reinstated after a
/// response. Per RFC 9112: HTTP/1.1 defaults to keep-alive unless
/// `Connection: close` is sent by either side; HTTP/1.0 defaults to close
/// unless `Connection: keep-alive` is present.
fn compute_reusable(
    client_asked_close: bool,
    resp_headers: &[(String, String)],
    minor: u8,
) -> bool {
    let server_says_close = header_contains_token(resp_headers, "connection", "close");
    let server_says_keepalive = header_contains_token(resp_headers, "connection", "keep-alive");
    if client_asked_close || server_says_close {
        false
    } else if minor >= 1 {
        true
    } else {
        server_says_keepalive
    }
}

/// Run a single buffered HTTP/1.1 request/response exchange on `stream`.
/// Returns the parsed response plus a `reusable` flag telling the caller
/// whether the stream may be reinstated in the pool.
async fn exchange_on_stream(
    stream: &mut dyn H1Io,
    method: &str,
    url: &url::Url,
    headers: Vec<(String, String)>,
    body: H1Body,
    target: H1Target,
) -> Result<(WireResponse, bool), H1PooledError> {
    let client_asked_close = send_h1_request(stream, method, url, headers, body, target).await?;
    let (status, resp_headers, resp_body, minor) = read_h1_response(stream, method).await?;
    let reusable = compute_reusable(client_asked_close, &resp_headers, minor);
    Ok((
        WireResponse {
            status,
            headers: resp_headers,
            body: resp_body,
        },
        reusable,
    ))
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

/// Read and parse the response head only, deciding the body framing but
/// leaving the body on the wire. Skips 1xx informational responses, the
/// same as [`read_h1_response`].
async fn read_h1_head<S>(stream: &mut S, method: &str) -> Result<H1Head, H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    loop {
        let mut buf = read_h1_headers(stream).await?;
        let header_end = find_header_end(&buf).ok_or_else(|| {
            H1PooledError::Http("HTTP/1.1 response missing header terminator".into())
        })?;
        let body_start = header_end + 4;
        let head = String::from_utf8_lossy(&buf[..header_end]);
        let (status, headers, minor) = parse_h1_head(&head)?;
        let initial_body = buf.split_off(body_start);

        if (100..200).contains(&status) && status != 101 {
            continue;
        }

        validate_framing_headers(&headers)?;

        let framing = if method.eq_ignore_ascii_case("HEAD") || matches!(status, 101 | 204 | 304) {
            BodyFraming::None
        } else if header_contains_token(&headers, "transfer-encoding", "chunked") {
            BodyFraming::Chunked
        } else if let Some(len) =
            header_first(&headers, "content-length").and_then(|v| v.trim().parse::<u64>().ok())
        {
            BodyFraming::Fixed(len)
        } else {
            BodyFraming::ToClose
        };

        return Ok(H1Head {
            status,
            headers,
            minor,
            framing,
            initial_body,
        });
    }
}

/// Send the request and read only the response head, leaving the body on the
/// wire for a streaming pump. Returns the head and whether the connection may
/// be reinstated after a clean full drain (a `ToClose` body delimits by EOF,
/// so its connection is spent and never reusable).
async fn exchange_head_on_stream(
    stream: &mut dyn H1Io,
    method: &str,
    url: &url::Url,
    headers: Vec<(String, String)>,
    body: H1Body,
    target: H1Target,
) -> Result<(H1Head, bool), H1PooledError> {
    let client_asked_close = send_h1_request(stream, method, url, headers, body, target).await?;
    let head = read_h1_head(stream, method).await?;
    let reusable = compute_reusable(client_asked_close, &head.headers, head.minor)
        && !matches!(head.framing, BodyFraming::ToClose);
    Ok((head, reusable))
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

/// Stream the response body to the consumer, then reinstate the connection on
/// a clean full drain (or drop it). The permit releases when this task ends.
async fn run_h1_stream_pump(mut pump: H1StreamPump) {
    let initial = std::mem::take(&mut pump.initial_body);
    let drained_clean = stream_body_into(pump.io.as_mut(), pump.framing, initial, &pump.tx).await;
    if drained_clean && pump.reusable {
        pump.pool
            .return_h1(pump.key, H1Slot { io: pump.io }, pump.tls);
        if pump.count_install {
            pump.pool.note_h1_install();
        }
    }
    // Otherwise the socket is dropped: an early consumer drop or a mid-body
    // error leaves unread/partial bytes on the wire, so the connection cannot
    // be safely reused.
    //
    // Release the per-host permit now that streaming has finished, freeing the
    // slot for a queued request. The permit is held purely for its `Drop`;
    // this makes the release point explicit.
    drop(pump.permit);
}

/// Drive the body into `tx` per `framing`. Returns `true` only on a clean
/// full drain; `false` if the consumer dropped the stream or an error
/// occurred (the error is forwarded to the consumer first).
async fn stream_body_into(
    stream: &mut dyn H1Io,
    framing: BodyFraming,
    initial: Vec<u8>,
    tx: &mpsc::Sender<io::Result<Bytes>>,
) -> bool {
    let result = match framing {
        BodyFraming::None => Ok(true),
        BodyFraming::Fixed(len) => stream_fixed_into(stream, initial, len, tx).await,
        BodyFraming::Chunked => stream_chunked_into(stream, initial, tx).await,
        BodyFraming::ToClose => stream_to_close_into(stream, initial, tx).await,
    };
    match result {
        Ok(clean) => clean,
        Err(e) => {
            let _ = tx.send(Err(e)).await;
            false
        }
    }
}

async fn stream_fixed_into(
    stream: &mut dyn H1Io,
    initial: Vec<u8>,
    len: u64,
    tx: &mpsc::Sender<io::Result<Bytes>>,
) -> io::Result<bool> {
    if len > MAX_H1_BODY_BYTES as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("HTTP/1.1 body exceeds {MAX_H1_BODY_BYTES} bytes"),
        ));
    }
    let mut remaining = len;
    if !initial.is_empty() {
        let take = (initial.len() as u64).min(remaining) as usize;
        if take > 0 {
            if tx
                .send(Ok(Bytes::copy_from_slice(&initial[..take])))
                .await
                .is_err()
            {
                return Ok(false);
            }
            remaining -= take as u64;
        }
    }
    let mut tmp = vec![0u8; 8192];
    while remaining > 0 {
        let want = remaining.min(tmp.len() as u64) as usize;
        let n = stream.read(&mut tmp[..want]).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed before HTTP/1.1 body completed",
            ));
        }
        if tx
            .send(Ok(Bytes::copy_from_slice(&tmp[..n])))
            .await
            .is_err()
        {
            return Ok(false);
        }
        remaining -= n as u64;
    }
    Ok(true)
}

async fn stream_to_close_into(
    stream: &mut dyn H1Io,
    initial: Vec<u8>,
    tx: &mpsc::Sender<io::Result<Bytes>>,
) -> io::Result<bool> {
    let mut total = initial.len();
    if total > MAX_H1_BODY_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("HTTP/1.1 body exceeds {MAX_H1_BODY_BYTES} bytes"),
        ));
    }
    if !initial.is_empty() && tx.send(Ok(Bytes::from(initial))).await.is_err() {
        return Ok(false);
    }
    let mut tmp = vec![0u8; 8192];
    loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Ok(true);
        }
        total += n;
        if total > MAX_H1_BODY_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("HTTP/1.1 body exceeds {MAX_H1_BODY_BYTES} bytes"),
            ));
        }
        if tx
            .send(Ok(Bytes::copy_from_slice(&tmp[..n])))
            .await
            .is_err()
        {
            return Ok(false);
        }
    }
}

async fn stream_chunked_into(
    stream: &mut dyn H1Io,
    mut buf: Vec<u8>,
    tx: &mpsc::Sender<io::Result<Bytes>>,
) -> io::Result<bool> {
    let mut total: usize = 0;
    loop {
        let line_end = read_until_crlf(stream, &mut buf)
            .await
            .map_err(h1err_to_io)?;
        let size_line = String::from_utf8_lossy(&buf[..line_end]);
        let size_token = size_line.split(';').next().unwrap_or("").trim();
        let size_u64 = u64::from_str_radix(size_token, 16).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid chunk size: {e}"),
            )
        })?;
        if size_u64 > MAX_H1_BODY_BYTES as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("HTTP/1.1 chunk size {size_u64} exceeds {MAX_H1_BODY_BYTES}-byte body cap"),
            ));
        }
        let size = size_u64 as usize;
        buf.drain(..line_end + 2);

        if size == 0 {
            read_chunk_trailers(stream, &mut buf)
                .await
                .map_err(h1err_to_io)?;
            return Ok(true);
        }

        read_until_available(stream, &mut buf, size + 2)
            .await
            .map_err(h1err_to_io)?;
        total += size;
        if total > MAX_H1_BODY_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("HTTP/1.1 body exceeds {MAX_H1_BODY_BYTES} bytes"),
            ));
        }
        if &buf[size..size + 2] != b"\r\n" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "chunk missing CRLF terminator",
            ));
        }
        let chunk = Bytes::copy_from_slice(&buf[..size]);
        buf.drain(..size + 2);
        if tx.send(Ok(chunk)).await.is_err() {
            return Ok(false);
        }
    }
}

/// Map a pool error to the `io::Error` the streaming consumer receives.
fn h1err_to_io(e: H1PooledError) -> io::Error {
    match e {
        H1PooledError::Io(io) => io,
        H1PooledError::ConnectionClosed(m) => io::Error::new(io::ErrorKind::UnexpectedEof, m),
        other => io::Error::new(io::ErrorKind::InvalidData, other.to_string()),
    }
}

/// Streaming variant of [`send_request_h1_pooled`]: reads the response head,
/// then hands the connection to a background pump that forwards body chunks
/// and reinstates the connection on a clean full drain. The permit moves into
/// the pump and releases when streaming ends.
#[allow(clippy::too_many_arguments)]
async fn send_request_h1_streaming(
    pool: &Arc<Pool>,
    connector: &ConnectorVariant,
    scheme: &str,
    host: &str,
    port: u16,
    method: &str,
    url: &url::Url,
    headers: Vec<(String, String)>,
    body: H1Body,
    proxy: Option<&str>,
    target: H1Target,
    permit: OwnedSemaphorePermit,
    key: PoolKey,
) -> Result<H1Response, H1PooledError> {
    let body_is_stream = body.is_stream();
    let retry_buf: Option<Bytes> = match &body {
        H1Body::Buffered(b) => Some(b.clone()),
        _ => None,
    };
    let mut body = body;

    // Try a pooled connection first, probing for socket-level liveness so a
    // stale keep-alive connection becomes a clean miss rather than a failed
    // exchange.
    if let Some((slot, tls)) = checkout_live_h1(pool, &key) {
        let pooled_body = std::mem::replace(&mut body, H1Body::Empty);
        let mut io = slot.io;
        match exchange_head_on_stream(
            io.as_mut(),
            method,
            url,
            headers.clone(),
            pooled_body,
            target,
        )
        .await
        {
            Ok((head, reusable)) => {
                tracing::Span::current().record("pool.hit", true);
                let (tx, rx) = mpsc::channel(STREAM_CHANNEL_DEPTH);
                tokio::spawn(run_h1_stream_pump(H1StreamPump {
                    io,
                    permit,
                    pool: pool.clone(),
                    key,
                    tls: tls.clone(),
                    framing: head.framing,
                    initial_body: head.initial_body,
                    reusable,
                    count_install: false,
                    tx,
                }));
                return Ok(H1Response {
                    status: head.status,
                    headers: head.headers,
                    body: H1ResponseBody::Streaming(BodyStream::new(rx)),
                    tls: tls_for_scheme(scheme, &tls),
                });
            }
            Err(e) => {
                tracing::info!(
                    target: "leyline::pool",
                    host = %key.host,
                    port = key.port,
                    error = %e,
                    "pool stale hit -- pooled h1 stream failed before response, opening fresh"
                );
                pool.note_h1_dead();
                if body_is_stream {
                    return Err(e);
                }
                if let Some(buf) = &retry_buf {
                    body = H1Body::Buffered(buf.clone());
                }
            }
        }
    }
    tracing::Span::current().record("pool.hit", false);

    // Miss — fresh connection under the same permit.
    let (io, tls): (Box<dyn H1Io>, TlsInfo) =
        open_new(connector, scheme, host, port, proxy).await?;
    let mut slot = H1Slot { io };
    let (head, reusable) =
        exchange_head_on_stream(slot.io.as_mut(), method, url, headers, body, target).await?;
    let (tx, rx) = mpsc::channel(STREAM_CHANNEL_DEPTH);
    tokio::spawn(run_h1_stream_pump(H1StreamPump {
        io: slot.io,
        permit,
        pool: pool.clone(),
        key,
        tls: tls.clone(),
        framing: head.framing,
        initial_body: head.initial_body,
        reusable,
        count_install: true,
        tx,
    }));
    Ok(H1Response {
        status: head.status,
        headers: head.headers,
        body: H1ResponseBody::Streaming(BodyStream::new(rx)),
        tls: tls_for_scheme(scheme, &tls),
    })
}

fn method_typically_has_body(method: &str) -> bool {
    ["POST", "PUT", "PATCH"]
        .iter()
        .any(|m| method.eq_ignore_ascii_case(m))
}

/// Parsed HTTP/1.x response head: status + headers + http/1.x minor
/// version (`1` for HTTP/1.1, `0` for HTTP/1.0).
type ParsedHead = (u16, Vec<(String, String)>, u8);

/// Parsed HTTP/1.x response: status + headers + body + http/1.x
/// minor version.
type ParsedResponse = (u16, Vec<(String, String)>, Vec<u8>, u8);

/// Returns `(status, headers, body, http_minor_version)`. The minor
/// version is `1` for HTTP/1.1 and `0` for HTTP/1.0.
async fn read_h1_response<S>(stream: &mut S, method: &str) -> Result<ParsedResponse, H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    loop {
        let mut buf = read_h1_headers(stream).await?;
        let header_end = find_header_end(&buf).ok_or_else(|| {
            H1PooledError::Http("HTTP/1.1 response missing header terminator".into())
        })?;
        let body_start = header_end + 4;
        let head = String::from_utf8_lossy(&buf[..header_end]);
        let (status, headers, minor) = parse_h1_head(&head)?;
        let initial_body = buf.split_off(body_start);

        // 1xx informational (except 101 Switching Protocols) — read again.
        if (100..200).contains(&status) && status != 101 {
            continue;
        }

        // RFC 9112 §6.1: reject multiple or conflicting framing
        // headers up-front. With a keep-alive pool an ambiguous
        // framing decision is a request-smuggling vector — the
        // parser and the server might disagree on where the body
        // ends, and the next pooled request lands in the wrong
        // place on the wire.
        validate_framing_headers(&headers)?;

        if method.eq_ignore_ascii_case("HEAD") || matches!(status, 101 | 204 | 304) {
            return Ok((status, headers, Vec::new(), minor));
        }

        let body = if header_contains_token(&headers, "transfer-encoding", "chunked") {
            read_chunked_body(stream, initial_body).await?
        } else if let Some(len) =
            header_first(&headers, "content-length").and_then(|v| v.trim().parse::<usize>().ok())
        {
            read_fixed_body(stream, initial_body, len).await?
        } else {
            read_to_close(stream, initial_body).await?
        };

        return Ok((status, headers, body, minor));
    }
}

/// RFC 9112 §6.1 framing validation.
///
/// Rejects response header sets with any of:
/// - Multiple `Content-Length` header lines (even if values agree) —
///   some servers/proxies concatenate into `CL: 10, 10` which real
///   clients will parse as "10" while intermediaries see the first,
///   creating a desync vector.
/// - Both `Content-Length` AND `Transfer-Encoding` present. RFC 9112
///   says TE wins, but the safe move with a keep-alive pool is to
///   refuse the connection entirely — request smuggling against
///   older intermediaries has shipped CVEs against every HTTP client
///   that accepted this combination.
/// - `Transfer-Encoding` where `chunked` is present but not the
///   final coding. RFC 9112: chunked MUST be last; otherwise body
///   length is undefined and the connection MUST close. Safer to
///   reject.
fn validate_framing_headers(headers: &[(String, String)]) -> Result<(), H1PooledError> {
    let cl_count = headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .count();
    if cl_count > 1 {
        return Err(H1PooledError::Http(
            "response has multiple Content-Length headers (RFC 9112 §6.1)".into(),
        ));
    }
    // A single CL header may still carry a comma-separated value —
    // that's the other flavour of the same attack.
    if let Some(v) = header_first(headers, "content-length") {
        if v.contains(',') {
            return Err(H1PooledError::Http(
                "response Content-Length contains multiple values".into(),
            ));
        }
        // RFC 9112 §8.6: Content-Length MUST be a non-negative decimal
        // integer. A present-but-unparseable value (`+10`, `10 foo`,
        // tab-prefixed, hex, anything but `[0-9]+`) must be rejected:
        // falling through to read-to-close is a smuggling vector when an
        // upstream parses leniently and disagrees on body length. Require
        // clean ASCII digits.
        let trimmed = v.trim();
        if trimmed.is_empty()
            || !trimmed.bytes().all(|b| b.is_ascii_digit())
            || trimmed.parse::<u64>().is_err()
        {
            return Err(H1PooledError::Http(format!(
                "response Content-Length `{v}` is not a valid decimal integer (RFC 9112 §8.6)"
            )));
        }
    }

    let te = header_first(headers, "transfer-encoding");
    if te.is_some() && cl_count > 0 {
        return Err(H1PooledError::Http(
            "response has both Content-Length and Transfer-Encoding (RFC 9112 §6.1)".into(),
        ));
    }
    if let Some(te) = te {
        // Last coding must be `chunked`. Split on commas, ignore
        // whitespace, compare last token.
        let last = te
            .split(',')
            .map(|t| t.trim())
            .rfind(|t| !t.is_empty())
            .unwrap_or("");
        if !last.eq_ignore_ascii_case("chunked") {
            return Err(H1PooledError::Http(format!(
                "response Transfer-Encoding `{te}`: `chunked` must be the final coding"
            )));
        }
    }
    Ok(())
}

async fn read_h1_headers<S>(stream: &mut S) -> Result<Vec<u8>, H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 2048];
    loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Err(H1PooledError::ConnectionClosed(
                "before HTTP/1.1 headers".into(),
            ));
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.len() > MAX_H1_HEADER_BYTES {
            return Err(H1PooledError::Http(format!(
                "HTTP/1.1 headers exceed {MAX_H1_HEADER_BYTES} bytes"
            )));
        }
        if find_header_end(&buf).is_some() {
            return Ok(buf);
        }
    }
}

fn parse_h1_head(head: &str) -> Result<ParsedHead, H1PooledError> {
    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| H1PooledError::Http("missing HTTP/1.1 status line".into()))?;
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or_default();
    if !version.starts_with("HTTP/1.") {
        return Err(H1PooledError::Http(format!(
            "invalid HTTP/1.1 status line: {status_line}"
        )));
    }
    let minor = match version.as_bytes().get(7) {
        Some(b'0') => 0,
        Some(b'1') => 1,
        _ => {
            return Err(H1PooledError::Http(format!(
                "unknown HTTP/1.x minor version in status line: {status_line}"
            )));
        }
    };
    let status = parts
        .next()
        .ok_or_else(|| H1PooledError::Http("missing HTTP status code".into()))?
        .parse::<u16>()
        .map_err(|e| H1PooledError::Http(format!("invalid HTTP status code: {e}")))?;

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        headers.push((name.trim().to_string(), value.trim_start().to_string()));
    }
    Ok((status, headers, minor))
}

async fn read_fixed_body<S>(
    stream: &mut S,
    mut body: Vec<u8>,
    len: usize,
) -> Result<Vec<u8>, H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    if len > MAX_H1_BODY_BYTES {
        return Err(H1PooledError::Http(format!(
            "HTTP/1.1 body exceeds {MAX_H1_BODY_BYTES} bytes"
        )));
    }
    while body.len() < len {
        let remaining = len - body.len();
        let mut tmp = vec![0u8; remaining.min(8192)];
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Err(H1PooledError::ConnectionClosed(
                "before HTTP/1.1 body completed".into(),
            ));
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(len);
    Ok(body)
}

async fn read_to_close<S>(stream: &mut S, mut body: Vec<u8>) -> Result<Vec<u8>, H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    let mut tmp = [0u8; 8192];
    loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Ok(body);
        }
        body.extend_from_slice(&tmp[..n]);
        if body.len() > MAX_H1_BODY_BYTES {
            return Err(H1PooledError::Http(format!(
                "HTTP/1.1 body exceeds {MAX_H1_BODY_BYTES} bytes"
            )));
        }
    }
}

async fn read_chunked_body<S>(stream: &mut S, mut buf: Vec<u8>) -> Result<Vec<u8>, H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    let mut out = Vec::new();
    loop {
        let line_end = read_until_crlf(stream, &mut buf).await?;
        let size_line = String::from_utf8_lossy(&buf[..line_end]);
        let size_token = size_line.split(';').next().unwrap_or("").trim();
        // Reject oversized chunk declarations up front: this guard
        // prevents a malicious peer from overflowing `size + 2` or
        // forcing an uncontrolled read.
        let size_u64 = u64::from_str_radix(size_token, 16)
            .map_err(|e| H1PooledError::Http(format!("invalid chunk size: {e}")))?;
        if size_u64 > MAX_H1_BODY_BYTES as u64 {
            return Err(H1PooledError::Http(format!(
                "HTTP/1.1 chunk size {size_u64} exceeds {MAX_H1_BODY_BYTES}-byte body cap"
            )));
        }
        let size = size_u64 as usize;
        buf.drain(..line_end + 2);

        if size == 0 {
            read_chunk_trailers(stream, &mut buf).await?;
            return Ok(out);
        }

        read_until_available(stream, &mut buf, size + 2).await?;
        out.extend_from_slice(&buf[..size]);
        if out.len() > MAX_H1_BODY_BYTES {
            return Err(H1PooledError::Http(format!(
                "HTTP/1.1 body exceeds {MAX_H1_BODY_BYTES} bytes"
            )));
        }
        if &buf[size..size + 2] != b"\r\n" {
            return Err(H1PooledError::Http("chunk missing CRLF terminator".into()));
        }
        buf.drain(..size + 2);
    }
}

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

async fn read_until_crlf<S>(stream: &mut S, buf: &mut Vec<u8>) -> Result<usize, H1PooledError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    loop {
        if let Some(pos) = buf.windows(2).position(|w| w == b"\r\n") {
            return Ok(pos);
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::{TcpListener, TcpStream};

    /// Connect a real loopback TCP pair and return (client, accepted server).
    async fn tcp_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        (client, server)
    }

    #[tokio::test]
    async fn live_idle_socket_probes_as_live() {
        let (mut client, _server) = tcp_pair().await;
        // Nothing sent by the peer: an idle keep-alive connection. The probe
        // must report it live (poll_read is Pending), not evict it.
        assert!(conn_is_live(&mut client as &mut dyn H1Io));
    }

    #[tokio::test]
    async fn peer_closed_socket_probes_as_dead() {
        let (mut client, server) = tcp_pair().await;
        // The peer drops its half.
        drop(server);
        // Wait for the FIN to land so the probe sees EOF deterministically.
        client.readable().await.unwrap();
        assert!(!conn_is_live(&mut client as &mut dyn H1Io));
    }

    #[tokio::test]
    async fn socket_with_pending_bytes_probes_as_dead() {
        let (mut client, mut server) = tcp_pair().await;
        // Unexpected bytes waiting before we sent a request = framing desync;
        // the connection must not be reused.
        server.write_all(b"x").await.unwrap();
        server.flush().await.unwrap();
        client.readable().await.unwrap();
        assert!(!conn_is_live(&mut client as &mut dyn H1Io));
    }

    #[tokio::test]
    async fn checkout_live_h1_drains_dead_and_counts_stale() {
        let pool = Arc::new(Pool::new());
        let key = make_key("127.0.0.1", 1, None, Transport::Tcp);

        let (client, server) = tcp_pair().await;
        // Peer closes, then wait for the FIN to land before pooling so the
        // checkout probe deterministically sees a dead socket.
        drop(server);
        client.readable().await.unwrap();
        pool.return_h1(
            key.clone(),
            H1Slot {
                io: Box::new(client),
            },
            TlsInfo::default(),
        );

        // The only pooled entry is dead: checkout drains it and reports a miss,
        // counting the catch as a probe catch (not a mid-exchange failure).
        assert!(checkout_live_h1(&pool, &key).is_none());
        let stats = pool.stats();
        assert_eq!(stats.stale_probed, 1, "probe catch must count as stale");
        assert_eq!(stats.evictions_dead, 0, "no mid-exchange failure occurred");
    }

    #[tokio::test]
    async fn checkout_live_h1_returns_a_live_connection_uncounted() {
        let pool = Arc::new(Pool::new());
        let key = make_key("127.0.0.1", 2, None, Transport::Tcp);

        // Keep the server end alive so the pooled connection stays open.
        let (client, _server) = tcp_pair().await;
        pool.return_h1(
            key.clone(),
            H1Slot {
                io: Box::new(client),
            },
            TlsInfo::default(),
        );

        assert!(
            checkout_live_h1(&pool, &key).is_some(),
            "a live pooled connection must be handed out"
        );
        assert_eq!(
            pool.stats().stale_probed,
            0,
            "a live connection is not a probe catch"
        );
    }
}
