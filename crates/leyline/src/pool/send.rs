use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::HttpVersion;
use crate::core::Body;
use crate::core::ResponseMode;
use crate::h2::client::{H2Client, H2ResponseEx, Head};
use crate::h2::config::H2Config;
use crate::h2::connection::{HeaderPair, PseudoHeaders};
use crate::h2::{ErrorCode, H2Error};
use crate::tls::FingerprintConnector;
use crate::trace;
use crate::util::is_idempotent;
use crate::{Cause, Error, ResponseTiming};

use super::connect::{checkout_h2_opened, open_h2};
use super::pool::Pool;
use super::types::{LegKind, Opened, PoolKey, TlsInfo, Transport};

#[cfg(feature = "http3")]
mod h3;
#[cfg(feature = "http3")]
pub(crate) use h3::{H3Request, send_request_h3_pooled};

enum Sent<R> {
    Done((R, TlsInfo, ResponseTiming)),
    Retry(Body),
}

#[tracing::instrument(
    name = "pool.send_request",
    level = "debug",
    skip_all,
    fields(
        http.authority = pseudo.authority.as_str(),
        http.method = pseudo.method.as_str(),
        pool.hit = tracing::field::Empty,
    )
)]
#[expect(
    clippy::too_many_arguments,
    reason = "flat per-request wire fields across one internal call path"
)]
pub(crate) async fn send_request(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    pseudo: PseudoHeaders,
    headers: Vec<HeaderPair>,
    body: Body,
    proxy: Option<&str>,
    mode: ResponseMode,
    opened: Option<Opened<H2Client>>,
) -> Result<(H2ResponseEx, TlsInfo, ResponseTiming), Error> {
    let started = opened.as_ref().map_or_else(Instant::now, |o| o.started);
    let head = Arc::new(Head { pseudo, headers });
    let default_port = if head.pseudo.scheme == "https" {
        443
    } else {
        80
    };
    let (host, port) = parse_authority(&head.pseudo.authority, default_port);
    let key = pool.key(&head.pseudo.scheme, host, port, proxy, Transport::Tcp);

    let mut opened = match opened {
        Some(opened) => {
            pool.evict_idle();
            opened
        }
        None => checkout_h2_opened(pool, connector, h2_config, key.clone())
            .await
            .map_err(|(e, _)| e)?,
    };
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
    opened: Opened<H2Client>,
    body: Body,
) -> Result<Sent<H2ResponseEx>, Error> {
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
    let send_started = Instant::now();
    trace::sent(
        ctx.host,
        ctx.head.pseudo.method.as_str(),
        ctx.head.pseudo.path.as_str(),
        HttpVersion::Http2,
        Duration::ZERO,
    );
    let resp = match opened
        .conn
        .send_shared(Arc::clone(ctx.head), body.into_h2(), ctx.mode)
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
    let send_ms = ResponseTiming::millis(send_started);
    trace::head(
        ctx.host,
        resp.status,
        HttpVersion::Http2,
        send_started.elapsed(),
        resp.headers.iter().cloned(),
    );
    let timing = ResponseTiming {
        reused: leg == LegKind::Pooled,
        connect_ms: opened.connect_ms,
        send_ms,
        total_ms: ResponseTiming::millis(ctx.started),
    };
    Ok(Sent::Done((resp, opened.tls, timing)))
}

fn resend_after_failure(ctx: &H2Send<'_>, e: H2Error, replay: Option<Body>) -> Result<Body, Error> {
    if !e.connection_failed() {
        return Err(Error::from(e));
    }
    tracing::info!(
        target: "leyline::pool",
        host = %ctx.key.host,
        port = ctx.key.port,
        proxied = ctx.key.proxy.is_some(),
        error = %e,
        "pool stale hit -- pooled h2 connection failed mid-request, opening fresh"
    );
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
    Error::new(Cause::Body)
        .with_message("pooled connection died and the streaming request body cannot be resent")
        .with_source(cause)
}

fn parse_authority(authority: &str, default_port: u16) -> (&str, u16) {
    if let Some(colon) = authority.rfind(':') {
        let port_str = &authority[colon + 1..];
        if let Ok(p) = port_str.parse::<u16>() {
            return (&authority[..colon], p);
        }
    }
    (authority, default_port)
}
