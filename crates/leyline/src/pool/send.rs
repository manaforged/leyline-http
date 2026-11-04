use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::core::Body;
use crate::core::ResponseMode;
use crate::core::transport::TransportResponse;
use crate::h2::client::{H2Client, Head};
use crate::h2::config::H2Config;
use crate::h2::connection::{HeaderPair, PseudoHeaders};
use crate::h2::{ErrorCode, H2Error};
use crate::tls::FingerprintConnector;
use crate::trace;
use crate::util::is_idempotent;
use crate::{Error, Kind, ResponseTiming};

use super::connect::{checkout_h2_opened, open_h2};
use super::liveness::confirm;
use super::pool::Pool;
use super::types::{LegKind, Opened, PoolKey, TlsInfo, Transport};

#[cfg(feature = "http3")]
mod h3;
#[cfg(feature = "http3")]
pub(crate) use h3::{H3Request, send_request_h3_pooled};

enum Sent<R> {
    Done(R),
    Retry(Body),
}

#[expect(
    clippy::too_many_arguments,
    reason = "flat per-request wire fields across one internal call path"
)]
pub(crate) async fn send_request(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    (host, port): (&str, u16),
    pseudo: PseudoHeaders,
    headers: Vec<HeaderPair>,
    body: Body,
    proxy: Option<&crate::core::ProxyUrl>,
    mode: ResponseMode,
    opened: Option<Opened<H2Client>>,
) -> Result<TransportResponse, Error> {
    let started = opened.as_ref().map_or_else(Instant::now, |o| o.started);
    let head = Arc::new(Head { pseudo, headers });
    let mut key = pool.key(&head.pseudo.scheme, host, port, proxy, Transport::Tcp);

    let mut opened = match opened {
        Some(opened) => {
            pool.evict_idle(Instant::now());
            opened
        }
        None => checkout_h2_opened(pool, connector, h2_config, &mut key)
            .await
            .map_err(|(e, _)| e)?,
    };
    key.lane = opened.lane;
    let ctx = H2Send {
        pool,
        head: &head,
        key: &key,
        host,
        port,
        mode,
        started,
    };
    let mut body = body;
    loop {
        match send_on(&ctx, opened, body).await? {
            Sent::Done(done) => return Ok(done),
            Sent::Retry(replay) => {
                let connect_started = Instant::now();
                let fresh = open_h2(pool, connector, h2_config, key.clone()).await?;
                opened = Opened::fresh(fresh, connect_started);
                body = replay;
            }
        }
    }
}

struct H2Send<'a> {
    pool: &'a Arc<Pool>,
    head: &'a Arc<Head>,
    key: &'a PoolKey,
    host: &'a str,
    port: u16,
    mode: ResponseMode,
    started: Instant,
}

async fn send_on(
    ctx: &H2Send<'_>,
    mut opened: Opened<H2Client>,
    body: Body,
) -> Result<Sent<TransportResponse>, Error> {
    let leg = opened.leg();
    let replay = match leg {
        LegKind::Pooled => {
            let replay = body.replay();
            trace::connect(ctx.host, ctx.port, true, Duration::ZERO);
            replay
        }
        LegKind::Fresh => {
            tracing::Span::current().record("pool.hit", false);
            None
        }
    };
    if let Some(probe) = opened.probe.take()
        && !(replay.is_some() && is_idempotent(&ctx.head.pseudo.method))
        && !confirm(ctx.pool, ctx.key, probe).await
    {
        return Ok(Sent::Retry(body));
    }
    let send_started = Instant::now();
    trace::sent(
        ctx.host,
        ctx.head.pseudo.method.as_str(),
        ctx.head.pseudo.path.as_str(),
        http::Version::HTTP_2,
        Duration::ZERO,
    );
    let resp = match opened
        .conn
        .send_reserved(Arc::clone(ctx.head), body, ctx.mode, opened.slot)
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            return match leg {
                LegKind::Pooled => resend_after_failure(ctx, e, replay).map(Sent::Retry),
                LegKind::Fresh => {
                    if e.connection_failed() {
                        ctx.pool.invalidate(ctx.key);
                    }
                    Err(Error::from(e))
                }
            };
        }
    };
    if leg == LegKind::Pooled {
        tracing::Span::current().record("pool.hit", true);
    }
    Ok(Sent::Done(answered(
        resp,
        ctx.host,
        opened.tls,
        opened.connect_ms,
        send_started,
        ctx.started,
    )))
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
    trace::head(
        host,
        resp.status.as_u16(),
        resp.version,
        waited,
        &resp.headers,
    );
    resp.tls = Some(tls);
    resp.timing = ResponseTiming {
        reused: connect_ms.is_none(),
        connect_ms,
        send_ms: ResponseTiming::span(waited),
        total_ms: ResponseTiming::span(answered.saturating_duration_since(started)),
    };
    resp
}

fn resend_after_failure(ctx: &H2Send<'_>, e: H2Error, replay: Option<Body>) -> Result<Body, Error> {
    if !e.connection_failed() {
        return Err(Error::from(e));
    }
    ctx.pool.stale_hit(ctx.key, http::Version::HTTP_2, &e);
    ctx.pool.invalidate(ctx.key);
    let refused = matches!(
        &e,
        H2Error::Stream {
            code: ErrorCode::RefusedStream,
            ..
        }
    );
    let allowed = refused || is_idempotent(&ctx.head.pseudo.method);
    resend(replay, allowed, e, not_resendable, Error::from)
}

pub(crate) fn resend<B, E, O>(
    replay: Option<B>,
    allowed: bool,
    error: E,
    unsendable: impl FnOnce(E) -> O,
    refused: impl FnOnce(E) -> O,
) -> Result<B, O> {
    match replay {
        None => Err(unsendable(error)),
        Some(_) if !allowed => Err(refused(error)),
        Some(body) => Ok(body),
    }
}

pub(crate) fn not_resendable(cause: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Error {
    Error::new(Kind::Body)
        .with_message("pooled connection died and the streaming request body cannot be resent")
        .with_source(cause)
}
