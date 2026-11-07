use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::Error;
use crate::core::transport::TransportResponse;
use crate::core::{Body, ResponseMode};
use crate::quic::H3Client;
use crate::trace;

use super::{Sent, not_resendable, resend};
use crate::pool::connect::{H3Target, checkout_h3_opened, open_h3};
use crate::pool::pool::Pool;
use crate::pool::types::{LegKind, Opened, PoolKey, Transport};

pub(crate) struct H3Request<'a> {
    pub(crate) method: &'a str,
    pub(crate) authority: &'a str,
    pub(crate) path: &'a str,
    pub(crate) headers: &'a [crate::h2::connection::HeaderPair],
    pub(crate) proxy: Option<&'a crate::core::ProxyUrl>,
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
) -> Result<TransportResponse, Error> {
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
                opened = open_h3(pool, target, key.clone()).await?;
                body = replay;
            }
        }
    }
}

async fn send_on(
    ctx: &H3Send<'_>,
    opened: Opened<H3Client>,
    body: Body,
) -> Result<Sent<TransportResponse>, Error> {
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
        http::Version::HTTP_3,
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
            ctx.pool
                .stale_hit(ctx.key, http::Version::HTTP_3, &e.message());
            ctx.pool.invalidate(ctx.key, &opened.conn);
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
    Ok(Sent::Done(super::answered(
        resp,
        ctx.host,
        opened.tls,
        opened.connect_ms,
        send_started,
        ctx.started,
    )))
}
