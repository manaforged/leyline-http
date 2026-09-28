use std::sync::Arc;

use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};

use crate::h2::config::H2Config;
use crate::pool::Pool;
use crate::profile::preset::HeaderPair;
use crate::tls::FingerprintConnector;

use crate::core::body::Body;
use crate::core::body_stream::BodyStream;
use crate::core::error::{Error, Kind, Result};
use crate::core::response::HttpVersion;

mod h1;
mod h2;

#[cfg(feature = "websocket")]
pub(crate) use h1::h1_error_to_core;
pub(crate) use h1::send_request_h1;

fn status(code: u16) -> Result<StatusCode> {
    StatusCode::from_u16(code)
        .map_err(|_| Error::new(Kind::Request).with_message(format!("invalid status code {code}")))
}

fn adopt<I, N, V>(headers: I) -> Vec<(HeaderName, HeaderValue)>
where
    I: IntoIterator<Item = (N, V)>,
    N: AsRef<[u8]>,
    V: Into<Bytes>,
{
    headers
        .into_iter()
        .filter_map(|(k, v)| {
            let name = HeaderName::from_bytes(k.as_ref()).ok()?;
            let value = HeaderValue::from_maybe_shared(v.into()).ok()?;
            Some((name, value))
        })
        .collect()
}

pub(crate) fn header_map<I, N, V>(headers: I) -> HeaderMap
where
    I: IntoIterator<Item = (N, V)>,
    N: AsRef<[u8]>,
    V: Into<Bytes>,
{
    adopt(headers).into_iter().collect()
}

pub(crate) enum TransportBody {
    Buffered(Vec<u8>),
    Streaming(BodyStream),
}

pub(crate) struct TransportResponse {
    pub(crate) status: StatusCode,
    pub(crate) headers: Vec<(HeaderName, HeaderValue)>,
    pub(crate) trailers: Vec<(HeaderName, HeaderValue)>,
    pub(crate) body: TransportBody,
    pub(crate) final_url: url::Url,
    pub(crate) version: HttpVersion,
    pub(crate) tls: Option<crate::pool::TlsInfo>,
    pub(crate) timing: crate::core::ResponseTiming,
}

pub(crate) struct Prepared<'a> {
    pub(crate) method: &'a str,
    pub(crate) url: &'a url::Url,
    pub(crate) headers: Vec<HeaderPair>,
    pub(crate) body: Body,
    pub(crate) proxy: Option<&'a str>,
    pub(crate) stream_response: bool,
}

#[tracing::instrument(
    name = "transport.auto",
    level = "debug",
    skip_all,
    fields(
        http.method = req.method,
        http.scheme = req.url.scheme(),
        http.host = req.url.host_str().unwrap_or(""),
        proxied = req.proxy.is_some(),
    )
)]
pub(crate) async fn send_request_auto(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    req: Prepared<'_>,
) -> Result<TransportResponse> {
    if req.url.scheme() == "http" {
        return Box::pin(send_request_h1(pool, connector, req)).await;
    }
    let host = req.url.host_str().unwrap_or("").to_string();
    let port = req.url.port_or_known_default().unwrap_or(443);
    let proxy_key = req.proxy.map(str::to_string);
    if pool.is_h1_only(&host, port, proxy_key.as_deref()) {
        return Box::pin(send_request_h1(pool, connector, req)).await;
    }
    let Prepared {
        method,
        url,
        headers,
        body,
        proxy,
        stream_response,
    } = req;

    let replay = body.replay();

    match send_request_h2(
        pool,
        connector,
        h2_config,
        Prepared {
            method,
            url,
            headers: headers.clone(),
            body,
            proxy,
            stream_response,
        },
    )
    .await
    {
        Ok(resp) => Ok(resp),
        Err(e) if is_h2_alpn_mismatch(&e) => {
            tracing::debug!(error = %e, "H2 ALPN mismatch, falling back to HTTP/1.1");
            pool.note_h1_only(&host, port, proxy_key.as_deref());
            let Some(body) = replay else {
                return Err(Error::new(Kind::Request).with_message(
                    "the origin negotiated HTTP/1.1 and a streaming request body cannot be \
                     replayed; send the request again, buffer the body, or use \
                     ProtocolPolicy::Http1",
                ));
            };
            Box::pin(send_request_h1(
                pool,
                connector,
                Prepared {
                    method,
                    url,
                    headers,
                    body,
                    proxy,
                    stream_response,
                },
            ))
            .await
        }
        Err(e) => Err(e),
    }
}

#[tracing::instrument(
    name = "transport.h2",
    level = "debug",
    skip_all,
    fields(http.method = req.method, http.host = req.url.host_str().unwrap_or(""))
)]
pub(crate) async fn send_request_h2(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    req: Prepared<'_>,
) -> Result<TransportResponse> {
    let Prepared {
        method,
        url,
        mut headers,
        body,
        proxy,
        stream_response,
    } = req;
    let pseudo = h2::request_pseudo(method, url)?;
    strip_connection_specific_headers(&mut headers)?;
    let (resp, tls, timing) = crate::pool::send_request(
        pool,
        connector,
        h2_config,
        pseudo,
        headers,
        body,
        proxy,
        stream_response,
    )
    .await?;
    h2::transport_response(resp, tls, timing, url)
}

pub(crate) fn check_framing(headers: &[HeaderPair]) -> Result<()> {
    let te = headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("transfer-encoding"))
        .count();
    let cl = headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .count();
    if te > 1 {
        return Err(Error::new(Kind::Config).with_message(format!(
            "{te} Transfer-Encoding headers on one request: framing would be ambiguous"
        )));
    }
    if te > 0 && cl > 0 {
        return Err(Error::new(Kind::Config).with_message(
            "Transfer-Encoding together with Content-Length on one request: framing would be ambiguous"
        ));
    }
    Ok(())
}

pub(crate) fn strip_connection_specific_headers(headers: &mut Vec<HeaderPair>) -> Result<()> {
    check_framing(headers)?;
    headers.retain_mut(|(name, value)| {
        if name.bytes().any(|b| b.is_ascii_uppercase()) {
            *name = name.to_lowercase().into();
        }
        match name.as_ref() {
            "transfer-encoding" | "connection" | "keep-alive" | "proxy-connection" | "upgrade"
            | "http2-settings" => false,
            "te" => value.eq_ignore_ascii_case("trailers"),
            _ => true,
        }
    });
    Ok(())
}

#[cfg(feature = "http3")]
#[tracing::instrument(
    name = "transport.h3",
    level = "debug",
    skip_all,
    fields(http.method = req.method, http.host = req.url.host_str().unwrap_or(""))
)]
pub(crate) async fn send_request_h3(
    pool: &Arc<Pool>,
    target: &crate::pool::H3Target<'_>,
    req: Prepared<'_>,
) -> Result<TransportResponse> {
    let Prepared {
        method,
        url,
        mut headers,
        body,
        stream_response,
        proxy,
        ..
    } = req;
    let host = url
        .host_str()
        .ok_or_else(|| Error::new(Kind::Config).with_message("no host in URL"))?;
    let port = url.port_or_known_default().unwrap_or(443);
    let authority = if url.scheme() == "https" && port == 443 {
        host.to_string()
    } else {
        format!("{host}:{port}")
    };
    let full_path = crate::util::request_target(url);

    strip_connection_specific_headers(&mut headers)?;

    let headers: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let (resp, tls) = crate::pool::send_request_h3_pooled(
        pool,
        target,
        host,
        port,
        crate::pool::H3Request {
            method,
            authority: &authority,
            path: &full_path,
            headers: &headers,
            proxy,
        },
        body,
        stream_response,
    )
    .await?;

    let transport_body = match resp.body {
        crate::quic::H3RespBody::Buffered(b) => TransportBody::Buffered(b),
        crate::quic::H3RespBody::Streaming(rx) => TransportBody::Streaming(BodyStream::new(rx)),
    };

    Ok(TransportResponse {
        status: status(resp.status)?,
        headers: adopt(resp.headers),
        trailers: adopt(resp.trailers),
        body: transport_body,
        final_url: url.clone(),
        version: HttpVersion::Http3,
        tls: Some(tls),
        timing: crate::core::ResponseTiming::default(),
    })
}

pub(crate) fn is_h2_alpn_mismatch(err: &Error) -> bool {
    err.alpn().is_some()
}

#[cfg(test)]
mod alpn_fallback_tests;
#[cfg(test)]
mod h2_header_hygiene_tests;
