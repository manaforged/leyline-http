use std::sync::Arc;

use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};

use crate::h2::client::H2Client;
use crate::h2::config::H2Config;
use crate::pool::h1::H1Dial;
use crate::pool::{H1Slot, Negotiated, Opened, Pool, negotiate};
use crate::profile::preset::HeaderPair;
use crate::tls::FingerprintConnector;

use crate::core::body::Body;
use crate::core::body_stream::BodyStream;
use crate::core::error::{Error, Kind, Result};
use crate::core::response::HttpVersion;

mod h1;
mod h2;
mod mode;

#[cfg(feature = "websocket")]
pub(crate) use h1::h1_error_to_core;
pub(crate) use h1::send_request_h1;
use h1::{H1Sent, send_h1_on};
pub use mode::{ErrorBudget, ResponseMode};

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
    pub(crate) response: ResponseMode,
}

struct Retry<'a> {
    method: &'a str,
    url: &'a url::Url,
    headers: Vec<HeaderPair>,
    proxy: Option<&'a str>,
    response: ResponseMode,
}

impl<'a> Prepared<'a> {
    fn split_replay(&self) -> (Retry<'a>, Option<Body>) {
        let retry = Retry {
            method: self.method,
            url: self.url,
            headers: self.headers.clone(),
            proxy: self.proxy,
            response: self.response,
        };
        (retry, self.body.replay())
    }
}

impl<'a> Retry<'a> {
    fn with_body(self, body: Body) -> Prepared<'a> {
        Prepared {
            method: self.method,
            url: self.url,
            headers: self.headers,
            body,
            proxy: self.proxy,
            response: self.response,
        }
    }
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
    let browser = H1Dial::Browser(h2_config);
    if req.url.scheme() == "http" {
        return Box::pin(send_request_h1(pool, connector, browser, req)).await;
    }
    let host = req.url.host_str().unwrap_or("").to_string();
    let port = req.url.port_or_known_default().unwrap_or(443);
    match negotiate(pool, connector, h2_config, &host, port, req.proxy).await? {
        Negotiated::H2(opened) => {
            send_h2_or_fall_back(pool, connector, h2_config, req, opened).await
        }
        Negotiated::H1(opened) => send_h1_or_upgrade(pool, connector, h2_config, req, opened).await,
    }
}

async fn send_h2_or_fall_back(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    req: Prepared<'_>,
    opened: Opened<H2Client>,
) -> Result<TransportResponse> {
    let (retry, replay) = req.split_replay();
    match send_h2_on(pool, connector, h2_config, req, Some(opened)).await {
        Err(e) if is_h2_alpn_mismatch(&e) => {
            tracing::debug!(error = %e, "H2 ALPN mismatch, falling back to HTTP/1.1");
            let Some(body) = replay else {
                return Err(Error::new(Kind::Request).with_message(
                    "the origin negotiated HTTP/1.1 and a streaming request body cannot be \
                     replayed; send the request again, buffer the body, or use \
                     ProtocolPolicy::Http1",
                ));
            };
            Box::pin(send_h1_or_upgrade(
                pool,
                connector,
                h2_config,
                retry.with_body(body),
                None,
            ))
            .await
        }
        other => other,
    }
}

async fn send_h1_or_upgrade(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    req: Prepared<'_>,
    opened: Option<Opened<H1Slot>>,
) -> Result<TransportResponse> {
    let (retry, _) = req.split_replay();
    let browser = H1Dial::Browser(h2_config);
    match Box::pin(send_h1_on(pool, connector, browser, req, opened)).await? {
        H1Sent::Done(resp) => Ok(*resp),
        H1Sent::Upgraded(opened, body) => {
            if let Some(host) = retry.url.host_str() {
                let port = retry.url.port_or_known_default().unwrap_or(443);
                pool.clear_h1_only(host, port, retry.proxy);
            }
            send_h2_on(
                pool,
                connector,
                h2_config,
                retry.with_body(body),
                Some(opened),
            )
            .await
        }
    }
}

pub(crate) async fn send_request_h2(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    req: Prepared<'_>,
) -> Result<TransportResponse> {
    send_h2_on(pool, connector, h2_config, req, None).await
}

#[tracing::instrument(
    name = "transport.h2",
    level = "debug",
    skip_all,
    fields(http.method = req.method, http.host = req.url.host_str().unwrap_or(""))
)]
async fn send_h2_on(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    req: Prepared<'_>,
    opened: Option<Opened<H2Client>>,
) -> Result<TransportResponse> {
    let Prepared {
        method,
        url,
        mut headers,
        body,
        proxy,
        response,
    } = req;
    let pseudo = h2::request_pseudo(method, url)?;
    strip_connection_specific_headers(&mut headers)?;
    let (resp, tls, timing) = crate::pool::send_request(
        pool, connector, h2_config, pseudo, headers, body, proxy, response, opened,
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
        sent_on(HttpVersion::Http2, name, value)
    });
    Ok(())
}

pub(crate) fn sent_on(version: HttpVersion, name: &str, value: &str) -> bool {
    match version {
        HttpVersion::Http1_1 => !name.eq_ignore_ascii_case("priority"),
        HttpVersion::Http2 | HttpVersion::Http3 => {
            if name.eq_ignore_ascii_case("te") {
                return value.eq_ignore_ascii_case("trailers");
            }
            !CONNECTION_SPECIFIC
                .iter()
                .any(|field| name.eq_ignore_ascii_case(field))
        }
    }
}

const CONNECTION_SPECIFIC: [&str; 7] = [
    "transfer-encoding",
    "connection",
    "keep-alive",
    "proxy-connection",
    "upgrade",
    "http2-settings",
    "host",
];

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
        response,
        proxy,
        ..
    } = req;
    let host = url
        .host_str()
        .ok_or_else(|| Error::new(Kind::Config).with_message("no host in URL"))?;
    let port = url.port_or_known_default().unwrap_or(443);
    let authority = crate::util::authority(url, host, port);
    let full_path = crate::util::request_target(url);

    strip_connection_specific_headers(&mut headers)?;

    let headers: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let (resp, tls, timing) = crate::pool::send_request_h3_pooled(
        pool,
        target,
        host,
        port,
        crate::pool::H3Request {
            method,
            authority: &authority,
            path: full_path,
            headers: &headers,
            proxy,
        },
        body,
        response,
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
        timing,
    })
}

pub(crate) fn is_h2_alpn_mismatch(err: &Error) -> bool {
    err.alpn()
        .is_some_and(|alpn| alpn.as_bytes() != crate::tls::alpn::H2)
}

#[cfg(test)]
mod alpn_fallback_tests;
#[cfg(test)]
mod h2_header_hygiene_tests;
