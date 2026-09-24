use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::HttpVersion;
use crate::core::Body;
use crate::h2::client::{H2ResponseEx, Head};
use crate::h2::config::H2Config;
use crate::h2::connection::{HeaderPair, PseudoHeaders};
use crate::h2::{ErrorCode, H2Error};
#[cfg(feature = "http3")]
use crate::quic::H3ResponseParts;
use crate::tls::FingerprintConnector;
use crate::trace;
use crate::util::is_idempotent;
use crate::{Error, Kind, ResponseTiming};

use super::connect::open_h2;
#[cfg(feature = "http3")]
use super::connect::{H3Target, open_h3};
use super::pool::Pool;
use super::types::{TlsInfo, Transport};
use super::{checkout_live_h2, make_key};

#[cfg(feature = "http3")]
pub(crate) struct H3Request<'a> {
    pub(crate) method: &'a str,
    pub(crate) authority: &'a str,
    pub(crate) path: &'a str,
    pub(crate) headers: &'a [(String, String)],
}

#[cfg(feature = "http3")]
pub(crate) async fn send_request_h3_pooled(
    pool: &Arc<Pool>,
    target: &H3Target<'_>,
    host: &str,
    port: u16,
    request: H3Request<'_>,
    body: Body,
    stream_response: bool,
) -> Result<(H3ResponseParts, TlsInfo), Error> {
    let key = make_key("https", host, port, None, Transport::Quic);

    pool.evict_idle();

    let replay = body.replay();
    let mut body = Some(body);

    if let Some((handle, tls)) = pool.checkout_h3(&key) {
        trace::connect(host, port, true, Duration::ZERO);
        let started = Instant::now();
        trace::sent(host, HttpVersion::Http3, Duration::ZERO);
        let (bytes, stream) = body.take().unwrap_or_default().into_parts();
        match handle
            .send_request(
                request.method,
                request.authority,
                request.path,
                request.headers,
                bytes,
                stream,
                stream_response,
            )
            .await
        {
            Ok(resp) => {
                trace::head(host, resp.status, HttpVersion::Http3, started.elapsed());
                return Ok((resp, tls));
            }
            Err(e) if !e.is_retryable() => {
                return Err(Error::new(Kind::Http3).with_message(e.message().to_string()));
            }
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
                    return Err(Error::new(Kind::Body).with_message(format!(
                        "pooled h3 connection died and a streaming request body cannot be retried: {}",
                        e.message()
                    )));
                };
                body = Some(replay);
            }
        }
    }

    let connect_started = Instant::now();
    let (handle, tls) = open_h3(pool, target, key).await?;
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
    trace::sent(host, HttpVersion::Http3, Duration::ZERO);
    let (bytes, stream) = body.unwrap_or_default().into_parts();
    let resp = handle
        .send_request(
            request.method,
            request.authority,
            request.path,
            request.headers,
            bytes,
            stream,
            stream_response,
        )
        .await
        .map_err(|e| Error::new(Kind::Http3).with_message(e.message().to_string()))?;
    trace::head(host, resp.status, HttpVersion::Http3, started.elapsed());
    Ok((resp, tls))
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
    stream_response: bool,
) -> Result<(H2ResponseEx, TlsInfo, ResponseTiming), Error> {
    let started = Instant::now();
    let head = Arc::new(Head { pseudo, headers });
    let pseudo = &head.pseudo;
    let host = &pseudo.authority;
    let port = if pseudo.scheme == "https" { 443 } else { 80 };

    let (connect_host, connect_port) = parse_authority(host, port);

    let key = make_key(
        &pseudo.scheme,
        connect_host,
        connect_port,
        proxy,
        Transport::Tcp,
    );

    pool.evict_idle();

    let replay = body.replay();
    let mut body = Some(body);

    if let Some((handle, tls)) = checkout_live_h2(pool, &key).await {
        trace::connect(connect_host, connect_port, true, Duration::ZERO);
        let pooled_body = body.take().unwrap_or_default().into_h2();
        let send_started = Instant::now();
        trace::sent(connect_host, HttpVersion::Http2, Duration::ZERO);
        match handle
            .send_shared(Arc::clone(&head), pooled_body, stream_response)
            .await
        {
            Ok(resp) => {
                tracing::Span::current().record("pool.hit", true);
                let send_ms = ms_since(send_started);
                trace::head(
                    connect_host,
                    resp.status,
                    HttpVersion::Http2,
                    send_started.elapsed(),
                );
                let timing = ResponseTiming {
                    reused: true,
                    connect_ms: None,
                    send_ms,
                    total_ms: ms_since(started),
                };
                return Ok((resp, tls, timing));
            }
            Err(e) => {
                tracing::info!(
                    target: "leyline::pool",
                    host = %key.host,
                    port = key.port,
                    proxied = key.proxy.is_some(),
                    error = %e,
                    "pool stale hit -- pooled h2 connection failed mid-request, opening fresh"
                );
                pool.invalidate(&key);
                let Some(replay) = replay else {
                    return Err(Error::new(Kind::Body).with_message(format!(
                        "pooled connection died and streaming body cannot be retried: {e}"
                    )));
                };
                let refused = matches!(
                    &e,
                    H2Error::Stream {
                        code: ErrorCode::RefusedStream,
                        ..
                    }
                );
                if !refused && !is_idempotent(&pseudo.method) {
                    return Err(Error::new(Kind::Http2).with_source(e));
                }
                body = Some(replay);
            }
        }
    }
    tracing::Span::current().record("pool.hit", false);

    let connect_started = Instant::now();
    let (handle, tls) = open_h2(pool, connector, h2_config, key.clone()).await?;
    let connect_ms = ms_since(connect_started);

    let send_started = Instant::now();
    let traced_host = trace::on().then(|| connect_host.to_string());
    trace::sent(connect_host, HttpVersion::Http2, Duration::ZERO);
    let body = body.unwrap_or_default().into_h2();
    let resp = match handle
        .send_shared(Arc::clone(&head), body, stream_response)
        .await
    {
        Ok(r) => {
            trace::head(
                traced_host.as_deref().unwrap_or_default(),
                r.status,
                HttpVersion::Http2,
                send_started.elapsed(),
            );
            r
        }
        Err(e) => {
            pool.invalidate(&key);
            return Err(Error::new(Kind::Http2).with_source(e));
        }
    };

    let timing = ResponseTiming {
        reused: false,
        connect_ms: Some(connect_ms),
        send_ms: ms_since(send_started),
        total_ms: ms_since(started),
    };
    Ok((resp, tls, timing))
}

fn ms_since(start: Instant) -> u32 {
    u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX)
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
