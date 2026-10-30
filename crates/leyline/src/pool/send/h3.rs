use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::core::{Body, ResponseMode};
use crate::quic::{H3Client, H3RespBody, H3Response};
use crate::trace;
use crate::{Error, HttpVersion, ResponseTiming};

use super::{Sent, not_resendable, resend};
use crate::pool::connect::{H3Target, checkout_h3_opened, open_h3};
use crate::pool::pool::Pool;
use crate::pool::types::{LegKind, Opened, PoolKey, TlsInfo, Transport};

pub(crate) struct H3Request<'a> {
    pub(crate) method: &'a str,
    pub(crate) authority: &'a str,
    pub(crate) path: &'a str,
    pub(crate) headers: &'a [(String, String)],
    pub(crate) proxy: Option<&'a str>,
}

struct H3Send<'a> {
    pool: &'a Arc<Pool>,
    key: &'a PoolKey,
    host: &'a str,
    port: u16,
    request: &'a H3Request<'a>,
    mode: ResponseMode,
    started: Instant,
}

pub(crate) async fn send_request_h3_pooled(
    pool: &Arc<Pool>,
    target: &H3Target<'_>,
    host: &str,
    port: u16,
    request: H3Request<'_>,
    body: Body,
    mode: ResponseMode,
) -> Result<(H3Response<H3RespBody>, TlsInfo, ResponseTiming), Error> {
    let started = Instant::now();
    let key = pool.key("https", host, port, request.proxy, Transport::Quic);
    let mut opened = checkout_h3_opened(pool, target, key.clone()).await?;
    let ctx = H3Send {
        pool,
        key: &key,
        host,
        port,
        request: &request,
        mode,
        started,
    };
    let mut body = body;
    loop {
        match send_on(&ctx, opened, body).await? {
            Sent::Done(done) => return Ok(done),
            Sent::Retry(replay) => {
                let connect_started = Instant::now();
                let fresh = open_h3(pool, target, key.clone()).await?;
                opened = Opened::fresh(fresh, connect_started);
                body = replay;
            }
        }
    }
}

async fn send_on(
    ctx: &H3Send<'_>,
    opened: Opened<H3Client>,
    body: Body,
) -> Result<Sent<H3Response<H3RespBody>>, Error> {
    let leg = opened.leg();
    let replay = match leg {
        LegKind::Pooled => {
            trace::connect(ctx.host, ctx.port, true, Duration::ZERO);
            body.replay()
        }
        LegKind::Fresh => {
            trace::connect(ctx.host, ctx.port, false, opened.started.elapsed());
            if trace::on() {
                trace::tls(
                    ctx.host,
                    opened.tls.version.as_deref(),
                    opened.tls.cipher.as_deref(),
                    Some("h3"),
                    Duration::ZERO,
                );
            }
            None
        }
    };
    let request = ctx.request;
    let send_started = Instant::now();
    trace::sent(
        ctx.host,
        request.method,
        request.path,
        HttpVersion::Http3,
        Duration::ZERO,
    );
    let (bytes, stream) = body.into_parts();
    let resp = match opened
        .conn
        .send_request(
            request.method,
            request.authority,
            request.path,
            request.headers,
            bytes,
            stream,
            ctx.mode,
        )
        .await
    {
        Ok(resp) => resp,
        Err(e) if leg == LegKind::Fresh || !e.is_retryable() => return Err(Error::from(e)),
        Err(e) => {
            tracing::info!(
                target: "leyline::pool",
                host = %ctx.key.host,
                port = ctx.key.port,
                error = %e.message(),
                "pool stale hit -- pooled h3 connection unsent, opening fresh"
            );
            ctx.pool.invalidate(ctx.key);
            return resend(
                replay,
                true,
                e,
                |e| not_resendable(e.message().into_owned()),
                Error::from,
            )
            .map(Sent::Retry);
        }
    };
    trace::head(
        ctx.host,
        resp.status,
        HttpVersion::Http3,
        send_started.elapsed(),
        resp.headers.iter().cloned(),
    );
    let timing = ResponseTiming {
        reused: leg == LegKind::Pooled,
        connect_ms: opened.connect_ms,
        send_ms: ResponseTiming::millis(send_started),
        total_ms: ResponseTiming::millis(ctx.started),
    };
    Ok(Sent::Done((resp, opened.tls, timing)))
}
