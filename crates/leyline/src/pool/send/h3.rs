use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::Instant;

use http::Method;

use crate::error::BodyLimit;
use crate::error::Error;
use crate::pool::Prepared;
use crate::quic::H3Client;
use crate::wire::Body;
use crate::wire::ResponseMode;
use crate::wire::TransportResponse;
use crate::wire::{HeaderPair, Scheme, strip_connection_specific_headers};

use super::{Attempt, LegTrace, Reopened, Sent, resending};
use crate::pool::Pool;
use crate::pool::H3Target;
use crate::pool::connect::{checkout_h3_opened, open_h3};
use crate::pool::types::{Opened, PoolKey};
use crate::profile::Carrier;

struct H3Request<'a> {
    method: &'a Method,
    authority: &'a str,
    path: &'a str,
    headers: &'a [HeaderPair],
}

struct H3Send<'a> {
    pool: &'a Arc<Pool>,
    key: &'a PoolKey,
    host: &'a str,
    request: &'a H3Request<'a>,
    mode: ResponseMode,
    body_limit: BodyLimit,
    started: Instant,
}

#[tracing::instrument(
    name = "transport.h3",
    level = "debug",
    skip_all,
    fields(http.method = req.method.as_str(), http.host = req.url.host_str().unwrap_or(""))
)]
pub(crate) async fn send_h3(
    pool: &Arc<Pool>,
    target: &H3Target<'_>,
    req: Prepared<'_>,
) -> Result<TransportResponse, Error> {
    let Prepared {
        method,
        url,
        headers,
        body,
        response,
        proxy,
        body_limit,
    } = req;
    let mut headers = headers.into_vec();
    let (host, port) = crate::wire::host_port(url)?;
    let authority = crate::wire::authority(url, host, port);
    let path = crate::util::request_target(url);

    strip_connection_specific_headers(&mut headers)?;

    let request = H3Request {
        method,
        authority: &authority,
        path,
        headers: &headers,
    };
    let started = Instant::now();
    let key = &pool.key(Scheme::Https, host, port, proxy, Carrier::Quic);
    let opened = checkout_h3_opened(pool, target, key.clone()).await?;
    let ctx = &H3Send {
        pool,
        key,
        host,
        request: &request,
        mode: response,
        body_limit,
        started,
    };
    let trace = LegTrace {
        host,
        port,
        method,
        path,
        version: http::Version::HTTP_3,
    };
    resending(
        &trace,
        Some(opened),
        body,
        move |opened, body, first| send_on(ctx, opened, body, first),
        move |body| reopen_h3(pool, target, key.clone(), body),
    )
    .await
}

async fn reopen_h3(
    pool: &Arc<Pool>,
    target: &H3Target<'_>,
    key: PoolKey,
    body: Body,
) -> Result<Reopened<H3Client, TransportResponse>, Error> {
    let opened = open_h3(pool, target, key).await?;
    Ok(ControlFlow::Continue((opened, body)))
}

async fn send_on(
    ctx: &H3Send<'_>,
    opened: Opened<H3Client>,
    body: Body,
    first: bool,
) -> Result<Sent<TransportResponse>, Error> {
    let leg = opened.leg();
    let replay = body.replay();
    let request = ctx.request;
    let send_started = Instant::now();
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
            ctx.body_limit,
        )
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            let attempt = Attempt {
                leg,
                first,
                method: request.method,
            };
            return ctx
                .pool
                .resend(ctx.key, &opened.conn, e.failure(), attempt, replay, e)
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
