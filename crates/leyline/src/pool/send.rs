use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::Instant;

use http::Method;

use super::Prepared;
use crate::error::BodyLimit;
use crate::error::{Error, Kind};
use crate::h2::H2Config;
use crate::h2::H2Error;
use crate::h2::client::{H2Client, Head};
use crate::h2::connection::PseudoHeaders;
use crate::header_str::HeaderStr;
use crate::tls::FingerprintConnector;
use crate::trace;
use crate::util::request_target;
use crate::wire::Body;
use crate::wire::ResponseMode;
use crate::wire::ResponseTiming;
use crate::wire::TransportResponse;
use crate::wire::{
    Failure, RequestHeaders, Scheme, h2_ready, is_idempotent, strip_connection_specific_headers,
};

use super::connect::{H2Leg, NotH2, checkout_h2, open_h2};
use super::liveness::confirm;
use super::pool::Pool;
use super::types::{LegKind, Opened, PoolKey};
use crate::profile::Carrier;
use crate::wire::TlsInfo;

#[cfg(feature = "http3")]
mod h3;
#[cfg(feature = "websocket")]
mod tunnel;
#[cfg(feature = "http3")]
pub(crate) use h3::send_h3;
#[cfg(feature = "websocket")]
pub(crate) use tunnel::open_extended_connect;

const STANDARD_METHODS: [&str; 9] = [
    "GET", "HEAD", "POST", "PUT", "DELETE", "OPTIONS", "PATCH", "TRACE", "CONNECT",
];

fn method_header(method: &Method) -> HeaderStr {
    let method = method.as_str();
    STANDARD_METHODS
        .iter()
        .copied()
        .find(|known| *known == method)
        .map_or_else(|| HeaderStr::from(method), HeaderStr::from_static)
}

fn request_pseudo(method: &Method, url: &url::Url) -> crate::error::Result<PseudoHeaders> {
    if Scheme::of(url) != Some(Scheme::Https) {
        return Err(Error::config("HTTP/2 requires an https:// URL"));
    }
    if !url.has_host() {
        return Err(Error::config("no host in URL"));
    }
    Ok(PseudoHeaders {
        method: method_header(method),
        scheme: HeaderStr::from_static("https"),
        authority: HeaderStr::from(&url[url::Position::BeforeHost..url::Position::AfterPort]),
        path: HeaderStr::from(request_target(url)),
        protocol: None,
    })
}

struct H2Parts<'a> {
    host: &'a str,
    port: u16,
    method: &'a Method,
    pseudo: crate::h2::connection::PseudoHeaders,
    headers: RequestHeaders,
    body: Body,
    proxy: Option<&'a crate::net::ProxyUrl>,
    mode: ResponseMode,
    body_limit: BodyLimit,
}

fn h2_parts(req: Prepared<'_>) -> crate::error::Result<H2Parts<'_>> {
    let Prepared {
        method,
        url,
        headers,
        body,
        proxy,
        response,
        body_limit,
    } = req;
    let pseudo = request_pseudo(method, url)?;
    let headers = if h2_ready(&headers) {
        headers
    } else {
        let mut headers = headers.into_vec();
        strip_connection_specific_headers(&mut headers)?;
        RequestHeaders::from(headers)
    };
    let (host, port) = crate::wire::host_port(url)?;
    Ok(H2Parts {
        host,
        port,
        method,
        pseudo,
        headers,
        body,
        proxy,
        mode: response,
        body_limit,
    })
}

pub(crate) enum Sent<R> {
    Done(R),
    Retry(Body),
}

#[derive(Clone, Copy)]
pub(crate) struct Attempt<'a> {
    pub(crate) leg: LegKind,
    pub(crate) first: bool,
    pub(crate) method: &'a Method,
}

pub(crate) struct LegTrace<'a> {
    pub(crate) host: &'a str,
    pub(crate) port: u16,
    pub(crate) method: &'a Method,
    pub(crate) path: &'a str,
    pub(crate) version: http::Version,
}

impl LegTrace<'_> {
    fn start(&self, leg: LegKind) {
        match leg {
            LegKind::Pooled => trace::connect(self.host, self.port, true),
            LegKind::Fresh => {
                tracing::Span::current().record("pool.hit", false);
            }
        }
        trace::sent(self.host, self.method.as_str(), self.path, self.version);
    }

    fn answered(leg: LegKind) {
        if leg == LegKind::Pooled {
            tracing::Span::current().record("pool.hit", true);
        }
    }
}

pub(crate) type Reopened<C, R> = ControlFlow<R, (Opened<C>, Body)>;

pub(crate) async fn resending<C, R, S, SF, O, OF>(
    trace: &LegTrace<'_>,
    opened: Option<Opened<C>>,
    body: Body,
    send: S,
    reopen: O,
) -> Result<R, Error>
where
    S: Fn(Opened<C>, Body, bool) -> SF,
    SF: Future<Output = Result<Sent<R>, Error>>,
    O: Fn(Body) -> OF,
    OF: Future<Output = Result<Reopened<C, R>, Error>>,
{
    let mut next = match opened {
        Some(opened) => ControlFlow::Continue((opened, body)),
        None => reopen(body).await?,
    };
    let mut first = true;
    loop {
        let (opened, body) = match next {
            ControlFlow::Continue(leg) => leg,
            ControlFlow::Break(done) => return Ok(done),
        };
        let leg = opened.leg();
        trace.start(leg);
        match send(opened, body, first).await? {
            Sent::Done(done) => {
                LegTrace::answered(leg);
                return Ok(done);
            }
            Sent::Retry(replay) => {
                next = reopen(replay).await?;
                first = false;
            }
        }
    }
}

/// An HTTP/2 exchange, or the HTTP/1.1 answer to its offer.
pub(crate) enum H2Outcome {
    Response(TransportResponse),
    Http1(NotH2),
}

impl H2Outcome {
    pub(crate) fn response(self) -> Result<TransportResponse, Error> {
        match self {
            Self::Response(response) => Ok(response),
            Self::Http1(not_h2) => Err(not_h2.error()),
        }
    }
}

pub(crate) async fn send_request(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    req: Prepared<'_>,
    opened: Option<Opened<H2Client>>,
) -> Result<H2Outcome, Error> {
    let H2Parts {
        host,
        port,
        method,
        pseudo,
        headers,
        body,
        proxy,
        mode,
        body_limit,
    } = h2_parts(req)?;
    let now = Instant::now();
    let started = opened.as_ref().map_or(now, |o| o.started);
    let head = Arc::new(Head {
        pseudo,
        headers,
        body_limit,
    });
    let mut key = pool.key(Scheme::Https, host, port, proxy, Carrier::Tcp);

    let opened = match opened {
        Some(opened) => {
            pool.evict_idle(now);
            opened
        }
        None => match checkout_h2(pool, connector, h2_config, &mut key, now).await? {
            H2Leg::Mux(opened) => opened,
            H2Leg::Http1 { not_h2, .. } => return Ok(H2Outcome::Http1(not_h2)),
        },
    };
    key.lane = opened.lane;
    let key = &key;
    let ctx = &H2Send {
        pool,
        head: &head,
        key,
        method,
        host,
        mode,
        started,
        now,
    };
    let trace = LegTrace {
        host,
        port,
        method,
        path: head.pseudo.path.as_str(),
        version: http::Version::HTTP_2,
    };
    resending(
        &trace,
        Some(opened),
        body,
        move |opened, body, first| send_on(ctx, opened, body, first),
        move |body| reopen_h2(pool, connector, h2_config, key.clone(), body),
    )
    .await
}

async fn reopen_h2(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    key: PoolKey,
    body: Body,
) -> Result<Reopened<H2Client, H2Outcome>, Error> {
    Ok(match open_h2(pool, connector, h2_config, key).await? {
        H2Leg::Mux(opened) => ControlFlow::Continue((opened, body)),
        H2Leg::Http1 { not_h2, .. } => ControlFlow::Break(H2Outcome::Http1(not_h2)),
    })
}

struct H2Send<'a> {
    pool: &'a Arc<Pool>,
    head: &'a Arc<Head>,
    key: &'a PoolKey,
    method: &'a Method,
    host: &'a str,
    mode: ResponseMode,
    started: Instant,
    now: Instant,
}

async fn send_on(
    ctx: &H2Send<'_>,
    mut opened: Opened<H2Client>,
    body: Body,
    first: bool,
) -> Result<Sent<H2Outcome>, Error> {
    let leg = opened.leg();
    let replay = body.replay();
    let mut send_started = match leg {
        LegKind::Pooled => ctx.now,
        LegKind::Fresh => Instant::now(),
    };
    if let Some(probe) = opened.probe.take()
        && !(leg == LegKind::Pooled && replay.is_some() && is_idempotent(ctx.method))
    {
        if !confirm(ctx.pool, ctx.key, &opened.conn, probe).await {
            return Ok(Sent::Retry(body));
        }
        send_started = Instant::now();
    }
    let resp = match opened
        .conn
        .send_reserved(Arc::clone(ctx.head), body, ctx.mode, opened.slot)
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            let attempt = Attempt {
                leg,
                first,
                method: ctx.method,
            };
            return ctx
                .pool
                .resend(ctx.key, &opened.conn, h2_failure(&e), attempt, replay, e)
                .map(Sent::Retry);
        }
    };
    Ok(Sent::Done(H2Outcome::Response(answered(
        resp,
        ctx.host,
        opened.tls,
        opened.connect_ms,
        send_started,
        ctx.started,
    ))))
}

pub(super) fn answered(
    mut resp: TransportResponse,
    host: &str,
    tls: TlsInfo,
    connect_ms: Option<u32>,
    send_started: Instant,
    started: Instant,
) -> TransportResponse {
    let answered = Instant::now();
    let waited = answered.saturating_duration_since(send_started);
    trace::head(host, resp.status.as_u16(), resp.version, &resp.headers);
    resp.tls = Some(tls);
    resp.timing = ResponseTiming {
        reused: connect_ms.is_none(),
        connect_ms,
        send_ms: ResponseTiming::span(waited),
        total_ms: ResponseTiming::span(answered.saturating_duration_since(started)),
    };
    resp
}

fn h2_failure(e: &H2Error) -> Failure {
    if matches!(e, H2Error::NotSent) {
        Failure::Unsent
    } else if e.refused() {
        Failure::Refused
    } else if e.connection_failed() {
        Failure::Connection
    } else {
        Failure::Request
    }
}

pub(crate) fn resend<E>(
    failure: Failure,
    attempt: Attempt<'_>,
    replay: Option<Body>,
    error: E,
    unsendable: impl FnOnce(E) -> Error,
    failed: impl FnOnce(E) -> Error,
) -> Result<Body, Error> {
    match (failure, replay) {
        (Failure::Refused, Some(body)) if attempt.first => Ok(body),
        (Failure::Request | Failure::Refused, _) => Err(failed(error)),
        _ if attempt.leg == LegKind::Fresh => Err(failed(error)),
        (_, None) => Err(unsendable(error)),
        (Failure::Connection, Some(_)) if !is_idempotent(attempt.method) => Err(failed(error)),
        (Failure::Unsent | Failure::Connection, Some(body)) => Ok(body),
    }
}

/// Logs a pooled connection that failed and returns whether the failure was one.
pub(crate) fn note_stale(
    pool: &Pool,
    key: &PoolKey,
    version: http::Version,
    failure: Failure,
    leg: LegKind,
    error: &dyn std::fmt::Display,
) -> bool {
    let stale = failure != Failure::Request && leg == LegKind::Pooled;
    if stale {
        pool.stale_hit(key, version, error);
    }
    stale
}

pub(crate) fn not_resendable(cause: impl crate::error::Cause) -> Error {
    Error::new(Kind::Body)
        .with_message("pooled connection died and the streaming request body cannot be resent")
        .with_cause(cause)
}

impl Pool {
    pub(crate) fn resend<C, E>(
        &self,
        key: &PoolKey,
        conn: &C,
        failure: Failure,
        attempt: Attempt<'_>,
        replay: Option<Body>,
        error: E,
    ) -> Result<Body, Error>
    where
        C: super::types::MuxClient,
        E: crate::error::Cause,
        Error: From<E>,
    {
        note_stale(self, key, C::VERSION, failure, attempt.leg, &error);
        if failure.retires_connection(C::VERSION) || conn.is_closed() {
            self.invalidate(key, conn);
        }
        resend(failure, attempt, replay, error, not_resendable, Error::from)
    }
}
