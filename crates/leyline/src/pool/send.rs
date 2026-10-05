use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::HttpVersion;
use crate::core::Body;
use crate::core::ResponseMode;
use crate::h2::client::{H2Client, H2ResponseEx, Head};
use crate::h2::config::H2Config;
use crate::h2::connection::{HeaderPair, PseudoHeaders};
use crate::h2::{ErrorCode, H2Error};
#[cfg(feature = "http3")]
use crate::quic::H3ResponseParts;
use crate::tls::FingerprintConnector;
use crate::trace;
use crate::util::is_idempotent;
use crate::{Error, Kind, ResponseTiming};

use super::checkout_live_h2;
use super::connect::open_h2;
#[cfg(feature = "http3")]
use super::connect::{H3Target, open_h3};
use super::pool::Pool;
use super::types::{Opened, PoolKey, TlsInfo, Transport};

#[cfg(feature = "http3")]
pub(crate) struct H3Request<'a> {
    pub(crate) method: &'a str,
    pub(crate) authority: &'a str,
    pub(crate) path: &'a str,
    pub(crate) headers: &'a [(String, String)],
    pub(crate) proxy: Option<&'a str>,
}

#[cfg(feature = "http3")]
pub(crate) async fn send_request_h3_pooled(
    pool: &Arc<Pool>,
    target: &H3Target<'_>,
    host: &str,
    port: u16,
    request: H3Request<'_>,
    body: Body,
    mode: ResponseMode,
) -> Result<(H3ResponseParts, TlsInfo, ResponseTiming), Error> {
    let request_started = Instant::now();
    let key = pool.key("https", host, port, request.proxy, Transport::Quic);

    pool.evict_idle();

    let replay = body.replay();
    let mut body = Some(body);

    if let Some((handle, tls)) = pool.checkout_h3(&key) {
        trace::connect(host, port, true, Duration::ZERO);
        let started = Instant::now();
        trace::sent(
            host,
            request.method,
            request.path,
            HttpVersion::Http3,
            Duration::ZERO,
        );
        let (bytes, stream) = body.take().unwrap_or_default().into_parts();
        match handle
            .send_request(
                request.method,
                request.authority,
                request.path,
                request.headers,
                bytes,
                stream,
                mode,
            )
            .await
        {
            Ok(resp) => {
                trace::head(
                    host,
                    resp.status,
                    HttpVersion::Http3,
                    started.elapsed(),
                    resp.headers.iter().cloned(),
                );
                let timing = ResponseTiming {
                    reused: true,
                    connect_ms: None,
                    send_ms: ResponseTiming::millis(started),
                    total_ms: ResponseTiming::millis(request_started),
                };
                return Ok((resp, tls, timing));
            }
            Err(e) if !e.is_retryable() => return Err(Error::from(e)),
            Err(e) => {
                tracing::info!(
                    target: "leyline::pool",
                    host = %key.host,
                    port = key.port,
                    error = %e.message(),
                    "pool stale hit -- pooled h3 connection unsent, opening fresh"
                );
                pool.invalidate(&key);
                let Some(replay) = replay else {
                    return Err(not_resendable(e.message().into_owned()));
                };
                body = Some(replay);
            }
        }
    }

    let connect_started = Instant::now();
    let (handle, tls) = open_h3(pool, target, key).await?;
    let connect_ms = ResponseTiming::millis(connect_started);
    trace::connect(host, port, false, connect_started.elapsed());
    if trace::on() {
        trace::tls(
            host,
            tls.version.as_deref(),
            tls.cipher.as_deref(),
            Some("h3"),
            Duration::ZERO,
        );
    }
    let started = Instant::now();
    trace::sent(
        host,
        request.method,
        request.path,
        HttpVersion::Http3,
        Duration::ZERO,
    );
    let (bytes, stream) = body.unwrap_or_default().into_parts();
    let resp = handle
        .send_request(
            request.method,
            request.authority,
            request.path,
            request.headers,
            bytes,
            stream,
            mode,
        )
        .await
        .map_err(Error::from)?;
    trace::head(
        host,
        resp.status,
        HttpVersion::Http3,
        started.elapsed(),
        resp.headers.iter().cloned(),
    );
    Ok((
        resp,
        tls,
        ResponseTiming {
            reused: false,
            connect_ms: Some(connect_ms),
            send_ms: ResponseTiming::millis(started),
            total_ms: ResponseTiming::millis(request_started),
        },
    ))
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

    pool.evict_idle();

    let opened = match opened {
        Some(opened) => Some(opened),
        None => checkout_live_h2(pool, &key).await.map(Opened::pooled),
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
    let body = match opened {
        Some(opened) if opened.connect_ms.is_some() => return send_fresh(&ctx, opened, body).await,
        Some(opened) => match send_pooled(&ctx, opened, body).await? {
            PooledSend::Done(done) => return Ok(done),
            PooledSend::Retry(body) => body,
        },
        None => body,
    };

    let connect_started = Instant::now();
    let fresh = open_h2(pool, connector, h2_config, key.clone()).await?;
    send_fresh(&ctx, Opened::fresh(fresh, connect_started), body).await
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

enum PooledSend {
    Done((H2ResponseEx, TlsInfo, ResponseTiming)),
    Retry(Body),
}

fn trace_sent(ctx: &H2Send<'_>) {
    trace::sent(
        ctx.host,
        ctx.head.pseudo.method.as_str(),
        ctx.head.pseudo.path.as_str(),
        HttpVersion::Http2,
        Duration::ZERO,
    );
}

async fn send_pooled(
    ctx: &H2Send<'_>,
    opened: Opened<H2Client>,
    body: Body,
) -> Result<PooledSend, Error> {
    let replay = body.replay();
    trace::connect(ctx.host, ctx.port, true, Duration::ZERO);
    let send_started = Instant::now();
    trace_sent(ctx);
    match opened
        .conn
        .send_shared(Arc::clone(ctx.head), body.into_h2(), ctx.mode)
        .await
    {
        Ok(resp) => {
            tracing::Span::current().record("pool.hit", true);
            let send_ms = ResponseTiming::millis(send_started);
            trace::head(
                ctx.host,
                resp.status,
                HttpVersion::Http2,
                send_started.elapsed(),
                resp.headers.iter().cloned(),
            );
            let timing = ResponseTiming {
                reused: true,
                connect_ms: None,
                send_ms,
                total_ms: ResponseTiming::millis(ctx.started),
            };
            Ok(PooledSend::Done((resp, opened.tls, timing)))
        }
        Err(e) => resend_after_failure(ctx, e, replay).map(PooledSend::Retry),
    }
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
    let Some(replay) = replay else {
        return Err(not_resendable(e));
    };
    let refused = matches!(
        &e,
        H2Error::Stream {
            code: ErrorCode::RefusedStream,
            ..
        }
    );
    if !refused && !is_idempotent(&ctx.head.pseudo.method) {
        return Err(Error::from(e));
    }
    Ok(replay)
}

async fn send_fresh(
    ctx: &H2Send<'_>,
    opened: Opened<H2Client>,
    body: Body,
) -> Result<(H2ResponseEx, TlsInfo, ResponseTiming), Error> {
    tracing::Span::current().record("pool.hit", false);
    let send_started = Instant::now();
    trace_sent(ctx);
    let resp = match opened
        .conn
        .send_shared(Arc::clone(ctx.head), body.into_h2(), ctx.mode)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            if e.connection_failed() {
                ctx.pool.invalidate(ctx.key);
            }
            return Err(Error::from(e));
        }
    };
    trace::head(
        ctx.host,
        resp.status,
        HttpVersion::Http2,
        send_started.elapsed(),
        resp.headers.iter().cloned(),
    );
    let timing = ResponseTiming {
        reused: false,
        connect_ms: opened.connect_ms,
        send_ms: ResponseTiming::millis(send_started),
        total_ms: ResponseTiming::millis(ctx.started),
    };
    Ok((resp, opened.tls, timing))
}

pub(crate) fn not_resendable(cause: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Error {
    Error::new(Kind::Body)
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
