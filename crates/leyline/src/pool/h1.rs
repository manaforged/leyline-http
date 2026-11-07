use std::borrow::Cow;
use std::convert::Infallible;
use std::io;
use std::ops::ControlFlow;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::StreamExt;
use http::Method;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{OwnedSemaphorePermit, mpsc};

use super::Prepared;
use crate::deadline::within;
use crate::error::BodyLimit;
use crate::error::{Error, Kind};
use crate::h2::H2Config;
use crate::tls::FingerprintConnector;
use crate::wire::Body;
use crate::wire::Failure;
use crate::wire::ResponseTiming;
use crate::wire::{ErrorBudget, ResponseMode};
use crate::wire::{Scheme, check_framing, is_idempotent};
use crate::wire::{TransportBody, TransportResponse};

use crate::pool::send::{
    Attempt, LegTrace, Reopened, Sent, not_resendable, note_stale, resend, resending,
};
use crate::pool::types::PoolKey;
use crate::pool::types::{Counter, LegKind, Opened};
use crate::pool::{H1Slot, Pool};
use crate::profile::Carrier;
use crate::wire::HeaderPair;
use crate::wire::RequestHeaders;
use crate::wire::TlsInfo;

pub trait H1Io: AsyncRead + AsyncWrite + Send + Unpin + 'static {}
impl<T> H1Io for T where T: AsyncRead + AsyncWrite + Send + Unpin + 'static {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum H1Target {
    OriginForm,
    AbsoluteForm,
}

/// A failed HTTP/1.1 exchange, classed where it failed.
#[derive(Debug)]
pub(crate) enum H1Error {
    Connection(Error),
    Request(Error),
}

impl H1Error {
    fn failure(&self) -> Failure {
        match self {
            Self::Connection(_) => Failure::Connection,
            Self::Request(_) => Failure::Request,
        }
    }

    pub(crate) fn into_error(self) -> Error {
        match self {
            Self::Connection(error) | Self::Request(error) => error,
        }
    }
}

impl From<io::Error> for H1Error {
    fn from(e: io::Error) -> Self {
        let error = Error::from(e);
        if error.cause() == Kind::Io
            && error.io().is_some()
            && !error.is_body_limit()
            && !is_malformed(&error)
        {
            Self::Connection(error)
        } else {
            Self::Request(error)
        }
    }
}

impl From<Error> for H1Error {
    fn from(error: Error) -> Self {
        Self::Request(error)
    }
}

pub(crate) fn closed(message: &str) -> H1Error {
    let source = io::Error::new(
        io::ErrorKind::UnexpectedEof,
        format!("connection closed: {message}"),
    );
    H1Error::Connection(Error::new(Kind::Io).with_source(source))
}

fn conn_is_live(io: &mut dyn H1Io, strict: bool) -> bool {
    use std::task::{Context, Poll};
    use tokio::io::ReadBuf;

    let mut probe = [0u8; 1];
    let mut buf = ReadBuf::new(&mut probe);
    let mut cx = Context::from_waker(std::task::Waker::noop());
    match crate::net::probing(strict, || Pin::new(io).poll_read(&mut cx, &mut buf)) {
        Poll::Pending => true,
        Poll::Ready(Ok(())) => false,
        Poll::Ready(Err(_)) => false,
    }
}

fn checkout_live_h1(
    pool: &Arc<Pool>,
    key: &PoolKey,
    strict: bool,
    now: Instant,
) -> Option<(H1Slot, TlsInfo)> {
    while let Some((mut slot, tls)) = pool.checkout_h1(key, now) {
        if conn_is_live(slot.io.as_mut(), strict) {
            return Some((slot, tls));
        }
        pool.note(Counter::StaleProbed);
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
    method: &str,
    url: &url::Url,
    headers: Vec<(String, String)>,
    body: Body,
    proxy: Option<&crate::net::ProxyUrl>,
    stream: bool,
) -> Result<TransportResponse, Error> {
    let method = Method::from_bytes(method.as_bytes()).map_err(|_| invalid_method(method))?;
    let req = Prepared {
        method: &method,
        url,
        headers: headers
            .into_iter()
            .map(|(name, value)| (name.into(), value.into()))
            .collect::<Vec<_>>()
            .into(),
        body,
        proxy,
        response: ResponseMode::from(stream),
        body_limit: BodyLimit::session(crate::wire::DEFAULT_MAX_BODY_SIZE),
    };
    send_http1(pool, connector, req)
        .await
        .map(|response| *response)
}

struct H1Request<'a> {
    scheme: Scheme,
    host: &'a str,
    port: u16,
    method: &'a Method,
    url: &'a url::Url,
    headers: RequestHeaders,
    proxy: Option<&'a crate::net::ProxyUrl>,
    target: H1Target,
    response: ResponseMode,
    body_limit: BodyLimit,
}

impl<'a> H1Request<'a> {
    fn new(req: Prepared<'a>) -> Result<(Self, Body), Error> {
        let Prepared {
            method,
            url,
            headers,
            body,
            proxy,
            response,
            body_limit,
        } = req;
        check_framing(&headers)?;
        let (host, port) = crate::wire::host_port(url)?;
        let scheme = Scheme::of(url).ok_or_else(|| {
            Error::config(format!(
                "unsupported URL scheme for HTTP/1.1: {}",
                url.scheme()
            ))
        })?;
        let (target, headers) = h1_target(scheme, proxy, headers);
        let request = Self {
            scheme,
            host,
            port,
            method,
            url,
            headers,
            proxy,
            target,
            response,
            body_limit,
        };
        Ok((request, body))
    }
}

fn h1_target(
    scheme: Scheme,
    proxy: Option<&crate::net::ProxyUrl>,
    mut headers: RequestHeaders,
) -> (H1Target, RequestHeaders) {
    let (target, credentials) = proxy_target(scheme, proxy);
    if target == H1Target::OriginForm && headers.h1_lines().is_some() {
        return (target, headers);
    }
    let pairs = headers.to_mut();
    pairs.retain(|(k, v)| crate::wire::sent_on(http::Version::HTTP_11, k, v));
    if let Some(credentials) = credentials {
        pairs.push((
            "Proxy-Authorization".into(),
            crate::header_str::HeaderStr::from(credentials),
        ));
    }
    (target, headers)
}

fn proxy_target(
    scheme: Scheme,
    proxy: Option<&crate::net::ProxyUrl>,
) -> (H1Target, Option<String>) {
    let Some(proxy_url) = proxy.filter(|_| scheme == Scheme::Http) else {
        return (H1Target::OriginForm, None);
    };
    let parsed = proxy_url.url();
    if proxy_url.scheme().is_socks() {
        return (H1Target::OriginForm, None);
    }
    (
        H1Target::AbsoluteForm,
        crate::util::proxy_basic_auth(parsed),
    )
}

pub(super) async fn send_h1<W: Copy + Send>(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    offer: Option<(&H2Config, W)>,
    req: Prepared<'_>,
    opened: Option<Opened<H1Slot>>,
) -> Result<H1Outcome<W>, Error> {
    let (req, body) = H1Request::new(req)?;
    let checked: &[HeaderPair] = match req.headers.h1_lines() {
        Some(_) => &[],
        None => &req.headers,
    };
    validate(req.method, checked)?;
    let key = pool.key(req.scheme, req.host, req.port, req.proxy, Carrier::Tcp);
    let permit = pool.acquire_h1_permit(&key).await;
    let now = Instant::now();
    pool.evict_idle(now);
    let resendable = !body.is_stream() && is_idempotent(req.method);
    let (started, first) = first_leg(pool, &key, opened, !resendable, now);
    let exchange = &Exchange {
        pool,
        key: &key,
        req: &req,
        started,
        parked: first.as_ref().is_some_and(|o| o.leg() == LegKind::Fresh),
    };
    let trace = LegTrace {
        host: req.host,
        port: req.port,
        method: req.method,
        path: crate::util::request_target(req.url),
        version: http::Version::HTTP_11,
    };
    let key = &key;
    let reply = resending(
        &trace,
        first,
        body,
        move |opened, body, first| exchange.send(opened, body, first),
        move |body| Box::pin(reopen(pool, key, connector, offer, body)),
    )
    .await?;
    match reply {
        Reply::Answered(delivered) => Ok(H1Outcome::Response(delivered.respond(permit))),
        Reply::Upgraded(upgrade) => {
            let Upgrade {
                opened,
                body,
                offered,
            } = *upgrade;
            Ok(H1Outcome::Upgraded {
                opened,
                body,
                _offered: offered,
            })
        }
    }
}

/// Sends one request over HTTP/1.1 on a dial that never offers h2.
pub(super) async fn send_http1(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    req: Prepared<'_>,
) -> Result<Box<TransportResponse>, Error> {
    let H1Outcome::Response(response) =
        send_h1(pool, connector, None::<(&H2Config, Infallible)>, req, None).await?;
    Ok(response)
}

async fn reopen<W: Copy + Send>(
    pool: &Pool,
    key: &PoolKey,
    connector: &FingerprintConnector,
    offer: Option<(&H2Config, W)>,
    body: Body,
) -> Result<Reopened<H1Slot, Reply<W>>, Error> {
    let started = Instant::now();
    Ok(match open_new(pool, key, connector, offer).await? {
        Dialed::H1(io, tls) => {
            ControlFlow::Continue((Opened::fresh((H1Slot::new(io), tls), started), body))
        }
        Dialed::H2(conn, tls, offered) => ControlFlow::Break(Reply::Upgraded(Box::new(Upgrade {
            opened: Opened::fresh((conn, tls), started),
            body,
            offered,
        }))),
    })
}

fn resend_after_failure(
    pool: &Pool,
    key: &PoolKey,
    attempt: Attempt<'_>,
    replay: Option<Body>,
    error: H1Error,
) -> Result<Body, Error> {
    let failure = error.failure();
    let error = error.into_error();
    if note_stale(
        pool,
        key,
        http::Version::HTTP_11,
        failure,
        attempt.leg,
        &error,
    ) {
        pool.note(Counter::EvictionsDead);
    }
    resend(
        failure,
        attempt,
        replay,
        error,
        |error| not_resendable(error.into_io()),
        |error| error,
    )
}

struct H1Head {
    status: u16,
    headers: crate::wire::Fields,
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

mod legs;
pub(crate) mod parse;
mod read;
mod streaming;
mod wire;
use crate::util::header_contains_token;
#[cfg(feature = "bench-internals")]
pub use wire::floor;

use crate::wire::h1::*;
use dial::{Dialed, open_new};
pub(super) use legs::H1Outcome;
use legs::{Reply, Upgrade, first_leg};
use parse::*;
use read::*;
use streaming::*;
use wire::*;

#[cfg(feature = "websocket")]
pub(crate) use wire::upgrade_on_stream;

#[cfg(test)]
mod tests;
