use std::borrow::Cow;
use std::io;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::StreamExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{OwnedSemaphorePermit, mpsc};

use crate::BodyStream;
use crate::tls::{FingerprintConnector, TlsError};
use crate::trace;
use crate::util::is_idempotent;

use crate::pool::types::PoolKey;
use crate::pool::types::Transport;
use crate::pool::{H1Slot, Pool, TlsInfo, make_key};

pub const MAX_H1_HEADER_BYTES: usize = 64 * 1024;

pub const MAX_H1_BODY_BYTES: usize = 100 * 1024 * 1024;

pub trait H1Io: AsyncRead + AsyncWrite + Send + Unpin + 'static {}
impl<T> H1Io for T where T: AsyncRead + AsyncWrite + Send + Unpin + 'static {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum H1Target {
    OriginForm,
    AbsoluteForm,
}

pub enum H1Body {
    Empty,
    Buffered(Bytes),
    FixedStream {
        stream: Pin<Box<dyn futures_util::Stream<Item = std::io::Result<Bytes>> + Send + 'static>>,
        length: u64,
    },
    ChunkedStream {
        stream: Pin<Box<dyn futures_util::Stream<Item = std::io::Result<Bytes>> + Send + 'static>>,
    },
}

pub enum H1ResponseBody {
    Buffered(Vec<u8>),
    Streaming(BodyStream),
}

pub struct H1Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: H1ResponseBody,
    pub tls: Option<TlsInfo>,
}

#[derive(Debug, thiserror::Error)]
pub enum H1PooledError {
    #[error("{0}")]
    Config(String),
    #[error(transparent)]
    Tls(#[from] TlsError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("http: {0}")]
    Http(String),
    #[error("connection closed: {0}")]
    ConnectionClosed(String),
}

fn conn_is_live(io: &mut dyn H1Io) -> bool {
    use std::task::{Context, Poll};
    use tokio::io::ReadBuf;

    let mut probe = [0u8; 1];
    let mut buf = ReadBuf::new(&mut probe);
    let mut cx = Context::from_waker(std::task::Waker::noop());
    match Pin::new(io).poll_read(&mut cx, &mut buf) {
        Poll::Pending => true,
        Poll::Ready(Ok(())) => false,
        Poll::Ready(Err(_)) => false,
    }
}

fn checkout_live_h1(pool: &Arc<Pool>, key: &PoolKey) -> Option<(H1Slot, TlsInfo)> {
    while let Some((mut slot, tls)) = pool.checkout_h1(key) {
        if conn_is_live(slot.io.as_mut()) {
            return Some((slot, tls));
        }
        pool.note_h1_stale_probed();
    }
    None
}

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

    let _permit = pool.acquire_h1_permit(&key).await;

    if stream {
        return send_request_h1_streaming(
            pool, connector, scheme, host, port, method, url, headers, body, proxy, target,
            _permit, key,
        )
        .await;
    }

    let replay = replay_body(method, &body);
    let mut body = body;

    if let Some((slot, tls)) = checkout_live_h1(pool, &key) {
        trace::connect(host, port, true, std::time::Duration::ZERO);
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
                pool.note_h1_dead();
                match replay {
                    Some(replay) => body = replay,
                    None => return Err(e),
                }
            }
        }
    }
    tracing::Span::current().record("pool.hit", false);

    let (io, tls): (Box<dyn H1Io>, TlsInfo) =
        open_new(connector, scheme, host, port, proxy).await?;

    let mut slot = H1Slot { io };
    let result = exchange_on_stream(slot.io.as_mut(), method, url, headers, body, target).await;

    match result {
        Ok((resp, reusable)) => {
            if reusable {
                pool.return_h1(key, slot, tls.clone());
                pool.note_h1_install();
            }
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

fn replay_body(method: &str, body: &H1Body) -> Option<H1Body> {
    if !is_idempotent(method) {
        return None;
    }
    match body {
        H1Body::Empty => Some(H1Body::Empty),
        H1Body::Buffered(b) => Some(H1Body::Buffered(b.clone())),
        _ => None,
    }
}

fn tls_for_scheme(scheme: &str, tls: &TlsInfo) -> Option<TlsInfo> {
    if scheme == "https" {
        Some(tls.clone())
    } else {
        None
    }
}

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
            let parsed = proxy
                .map(url::Url::parse)
                .transpose()
                .map_err(|e| H1PooledError::Config(format!("invalid proxy URL: {e}")))?;
            let (dial_host, dial_port) = match &parsed {
                Some(parsed) if parsed.scheme() != "http" => {
                    return Err(H1PooledError::Config(
                        "plaintext HTTP currently supports http:// proxies only".into(),
                    ));
                }
                Some(parsed) => (
                    parsed
                        .host_str()
                        .ok_or_else(|| H1PooledError::Config("proxy has no host".into()))?,
                    parsed.port_or_known_default().unwrap_or(8080),
                ),
                None => (host, port),
            };
            let stream = connector
                .with_timeout(connector.dial_tcp(dial_host, dial_port))
                .await?;
            let io: Box<dyn H1Io> = Box::new(stream);
            Ok((io, TlsInfo::default()))
        }
        other => Err(H1PooledError::Config(format!(
            "unsupported URL scheme for HTTP/1.1: {other}"
        ))),
    }
}

struct WireResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

const STREAM_CHANNEL_DEPTH: usize = 16;

#[derive(Debug, Clone, Copy)]
enum BodyFraming {
    None,
    Fixed(u64),
    Chunked,
    ToClose,
}

struct H1Head {
    status: u16,
    headers: Vec<(String, String)>,
    minor: u8,
    framing: BodyFraming,
    initial_body: Vec<u8>,
}

struct H1StreamPump {
    io: Box<dyn H1Io>,
    permit: OwnedSemaphorePermit,
    pool: Arc<Pool>,
    key: PoolKey,
    tls: TlsInfo,
    framing: BodyFraming,
    initial_body: Vec<u8>,
    reusable: bool,
    count_install: bool,
    tx: mpsc::Sender<io::Result<Bytes>>,
}

type ParsedHead = (u16, Vec<(String, String)>, u8);

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

fn is_valid_token(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    s.bytes().all(|b| {
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

fn is_valid_header_value(s: &str) -> bool {
    s.bytes()
        .all(|b| matches!(b, b'\t' | b' '..=b'~' | 0x80..=0xFF))
}

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

pub(crate) mod parse;
mod streaming;
mod wire;
use parse::*;
use streaming::*;
use wire::*;

#[cfg(test)]
mod tests;
