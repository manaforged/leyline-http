use std::borrow::Cow;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{OwnedSemaphorePermit, mpsc};

use crate::BodyStream;
use crate::ResponseTiming;
use crate::core::Body;
use crate::core::deadline::within;
use crate::core::transport::{TransportBody, TransportResponse};
use crate::core::{ErrorBudget, ResponseMode};
use crate::tls::FingerprintConnector;
use crate::trace;
use crate::util::is_idempotent;
use crate::{Error, Kind};

use crate::pool::send::{not_resendable, resend};
use crate::pool::types::PoolKey;
use crate::pool::types::{Opened, Transport};
use crate::pool::{H1Slot, Pool, TlsInfo};
use crate::profile::preset::HeaderPair;

pub const MAX_H1_HEADER_BYTES: usize = 64 * 1024;

pub trait H1Io: AsyncRead + AsyncWrite + Send + Unpin + 'static {}
impl<T> H1Io for T where T: AsyncRead + AsyncWrite + Send + Unpin + 'static {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum H1Target {
    OriginForm,
    AbsoluteForm,
}

use http::HeaderMap;

pub(crate) fn malformed(message: impl Into<String>) -> Error {
    let message = message.into();
    let source = io::Error::new(io::ErrorKind::InvalidData, format!("http: {message}"));
    Error::new(Kind::Io)
        .with_message(message)
        .with_source(source)
}

pub(crate) fn closed(message: &str) -> Error {
    let source = io::Error::new(
        io::ErrorKind::UnexpectedEof,
        format!("connection closed: {message}"),
    );
    Error::new(Kind::Io).with_source(source)
}

fn connection_failed(error: &Error) -> bool {
    error.cause() == Kind::Io
        && error.message().is_none()
        && error.io().is_some()
        && !error.is_body_limit()
}

fn conn_is_live(io: &mut dyn H1Io, strict: bool) -> bool {
    use std::task::{Context, Poll};
    use tokio::io::ReadBuf;

    let mut probe = [0u8; 1];
    let mut buf = ReadBuf::new(&mut probe);
    let mut cx = Context::from_waker(std::task::Waker::noop());
    match crate::tls::probing(strict, || Pin::new(io).poll_read(&mut cx, &mut buf)) {
        Poll::Pending => true,
        Poll::Ready(Ok(())) => false,
        Poll::Ready(Err(_)) => false,
    }
}

fn checkout_live_h1(
    pool: &Arc<Pool>,
    key: &PoolKey,
    strict: bool,
    now: std::time::Instant,
) -> Option<(H1Slot, TlsInfo)> {
    while let Some((mut slot, tls)) = pool.checkout_h1(key, now) {
        if conn_is_live(slot.io.as_mut(), strict) {
            return Some((slot, tls));
        }
        pool.note_h1_stale_probed();
    }
    None
}

#[cfg(feature = "bench-internals")]
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
    body: Body,
    proxy: Option<&crate::core::ProxyUrl>,
    target: H1Target,
    stream: bool,
) -> Result<TransportResponse, Error> {
    let request = H1Request {
        scheme,
        host,
        port,
        method,
        url,
        headers: headers
            .into_iter()
            .map(|(name, value)| (name.into(), value.into()))
            .collect(),
        proxy,
        target,
        response: ResponseMode::from(stream),
    };
    send_h1_pooled(pool, connector, request, body, None, H1Dial::Http1Only)
        .await?
        .response()
}

pub(crate) struct H1Request<'a> {
    pub(crate) scheme: &'a str,
    pub(crate) host: &'a str,
    pub(crate) port: u16,
    pub(crate) method: &'a str,
    pub(crate) url: &'a url::Url,
    pub(crate) headers: Vec<HeaderPair>,
    pub(crate) proxy: Option<&'a crate::core::ProxyUrl>,
    pub(crate) target: H1Target,
    pub(crate) response: ResponseMode,
}

pub(crate) async fn send_h1_pooled(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    req: H1Request<'_>,
    mut body: Body,
    opened: Option<Opened<H1Slot>>,
    dial: H1Dial<'_>,
) -> Result<H1Outcome, Error> {
    validate(req.method, &req.headers)?;
    let key = pool.key(req.scheme, req.host, req.port, req.proxy, Transport::Tcp);
    let permit = pool.acquire_h1_permit(&key).await;
    let now = std::time::Instant::now();
    pool.evict_idle(now);
    let replay = body.replay();
    let resendable = replay.is_some() && is_idempotent(req.method);
    let legs = first_legs(pool, &key, opened, !resendable, now);
    let started = legs.started;
    let mut exchange = Exchange {
        pool,
        key: &key,
        req: &req,
        permit: Some(permit),
    };

    if let Some((slot, tls)) = legs.pooled {
        trace::connect(req.host, req.port, true, Duration::ZERO);
        match exchange
            .on(slot, &tls, std::mem::take(&mut body), false)
            .await
        {
            Ok(head) => {
                tracing::Span::current().record("pool.hit", true);
                return head.respond(req.scheme, tls, ResponseTiming::leg(started, None));
            }
            Err(e) => body = resend_after_failure(pool, &key, req.method, replay, e)?,
        }
    }
    tracing::Span::current().record("pool.hit", false);
    let installed = legs.fresh.is_some();
    let opened = match Box::pin(fresh_leg(pool, &key, connector, dial, legs.fresh)).await? {
        Leg::H1(opened) => opened,
        Leg::H2(opened) => return Ok(H1Outcome::Upgraded { opened, body }),
    };
    let connect_ms = opened.connect_ms.unwrap_or_default();
    let head = exchange
        .on(opened.conn, &opened.tls, body, !installed)
        .await?;
    head.respond(
        req.scheme,
        opened.tls,
        ResponseTiming::leg(started, Some(connect_ms)),
    )
}

fn resend_after_failure(
    pool: &Pool,
    key: &PoolKey,
    method: &str,
    replay: Option<Body>,
    error: Error,
) -> Result<Body, Error> {
    if !connection_failed(&error) {
        return Err(error);
    }
    pool.stale_hit(key, http::Version::HTTP_11, &error);
    pool.note_h1_dead();
    resend(
        replay,
        is_idempotent(method),
        error,
        |error| not_resendable(error.into_io()),
        |error| error,
    )
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
    headers: HeaderMap,
    minor: u8,
    framing: BodyFraming,
    initial_body: Vec<u8>,
}

struct H1StreamPump {
    io: Box<dyn H1Io>,
    permit: Option<OwnedSemaphorePermit>,
    pool: Arc<Pool>,
    key: PoolKey,
    tls: TlsInfo,
    framing: BodyFraming,
    initial_body: Vec<u8>,
    reusable: bool,
    count_install: bool,
    drain_limit: u64,
    tx: mpsc::Sender<io::Result<Bytes>>,
}

mod dial;
mod headers;
mod legs;
pub(crate) mod parse;
mod read;
mod streaming;
mod wire;
use crate::util::{has_token, header_contains_token};
#[cfg(feature = "bench-internals")]
pub use wire::floor;

fn text(headers: &HeaderMap, name: http::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}
pub(crate) use dial::H1Dial;
use headers::*;
pub(crate) use legs::H1Outcome;
use legs::{Leg, first_legs, fresh_leg};
use parse::*;
use read::*;
use streaming::*;
use wire::*;

#[cfg(feature = "websocket")]
pub(crate) use wire::upgrade_on_stream;

#[cfg(test)]
mod tests;
