use std::borrow::Cow;
use std::io;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::StreamExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{OwnedSemaphorePermit, mpsc};

use crate::BodyStream;
use crate::ResponseTiming;
use crate::core::session::decompress::BodyLimit;
use crate::tls::{FingerprintConnector, TlsError};
use crate::trace;
use crate::util::is_idempotent;
use std::time::Instant;

use crate::pool::send::not_resendable;
use crate::pool::types::PoolKey;
use crate::pool::types::Transport;
use crate::pool::{H1Slot, Pool, TlsInfo, make_key};

pub const MAX_H1_HEADER_BYTES: usize = 64 * 1024;

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
    pub timing: ResponseTiming,
}

#[derive(Debug, thiserror::Error)]
pub enum H1PooledError {
    #[error("{0}")]
    Config(String),
    #[error(transparent)]
    Tls(#[from] TlsError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("request body stream failed")]
    RequestBody(#[source] std::io::Error),
    #[error("http: {0}")]
    Http(String),
    #[error("connection closed: {0}")]
    ConnectionClosed(String),
    #[error(transparent)]
    NotResendable(crate::Error),
}

impl H1PooledError {
    fn connection_failed(&self) -> bool {
        match self {
            Self::Io(io) => BodyLimit::of_io(io).is_none(),
            Self::ConnectionClosed(_) => true,
            _ => false,
        }
    }
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
    validate(method, &headers)?;
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

    let started = Instant::now();
    let replay = replay_body(&body);
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
            pool.max_body_size,
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
                    timing: ResponseTiming::leg(started, None),
                });
            }
            Err(e) => body = resend_after_failure(pool, &key, method, replay, e)?,
        }
    }
    tracing::Span::current().record("pool.hit", false);

    let connect_started = Instant::now();
    let (io, tls): (Box<dyn H1Io>, TlsInfo) =
        open_new(connector, scheme, host, port, proxy).await?;
    let connect_ms = ResponseTiming::millis(connect_started);

    let mut slot = H1Slot { io };
    let result = exchange_on_stream(
        slot.io.as_mut(),
        method,
        url,
        headers,
        body,
        target,
        pool.max_body_size,
    )
    .await;

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
                timing: ResponseTiming::leg(started, Some(connect_ms)),
            })
        }
        Err(e) => Err(e),
    }
}

fn replay_body(body: &H1Body) -> Option<H1Body> {
    match body {
        H1Body::Empty => Some(H1Body::Empty),
        H1Body::Buffered(b) => Some(H1Body::Buffered(b.clone())),
        _ => None,
    }
}

fn resend_after_failure(
    pool: &Pool,
    key: &PoolKey,
    method: &str,
    replay: Option<H1Body>,
    error: H1PooledError,
) -> Result<H1Body, H1PooledError> {
    if !error.connection_failed() {
        return Err(error);
    }
    tracing::info!(
        target: "leyline::pool",
        host = %key.host,
        port = key.port,
        proxied = key.proxy.is_some(),
        error = %error,
        "pool stale hit -- pooled h1 connection failed, opening fresh"
    );
    pool.note_h1_dead();
    match replay {
        None => Err(H1PooledError::NotResendable(not_resendable(h1err_to_io(
            error,
        )))),
        Some(_) if !is_idempotent(method) => Err(error),
        Some(body) => Ok(body),
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
        "http" => Ok((
            dial_plain(connector, host, port, proxy).await?,
            TlsInfo::default(),
        )),
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

const MAX_H1_INFORMATIONAL: usize = 16;

mod dial;
mod headers;
pub(crate) mod parse;
mod read;
mod streaming;
mod wire;
use dial::dial_plain;
use headers::*;
use parse::*;
use read::*;
use streaming::*;
use wire::*;

pub(crate) use wire::h1err_to_io;

#[cfg(feature = "websocket")]
pub(crate) use wire::upgrade_on_stream;

#[cfg(test)]
mod tests;
