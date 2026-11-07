use std::sync::Arc;

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
use crate::core::request_headers::RequestHeaders;

mod h1;
mod h2;
mod mode;
mod response;

pub(crate) use response::ResponseState;

use crate::pool::h1::H1Outcome;
pub(crate) use h1::send_h1;
pub use mode::{ErrorBudget, ResponseMode};

pub(crate) fn status(code: u16) -> Result<StatusCode> {
    StatusCode::from_u16(code).map_err(|e| {
        Error::new(Kind::Request)
            .with_message(format!("invalid status code {code}"))
            .with_source(e)
    })
}

pub(crate) fn trailers(fields: HeaderMap) -> Option<Box<HeaderMap>> {
    (!fields.is_empty()).then(|| Box::new(fields))
}

#[derive(Default)]
pub(crate) struct ResponseHead {
    status: Option<StatusCode>,
    regular: bool,
    headers: HeaderMap,
    length: Option<u64>,
}

pub(crate) fn response_field(
    name: &[u8],
    value: &[u8],
    typed: Option<(HeaderName, HeaderValue)>,
) -> std::result::Result<(HeaderName, HeaderValue), &'static str> {
    if name.iter().any(u8::is_ascii_uppercase)
        || value.iter().any(|byte| matches!(byte, 0 | b'\r' | b'\n'))
        || matches!(value.first(), Some(b' ' | b'\t'))
        || matches!(value.last(), Some(b' ' | b'\t'))
    {
        return Err("response contains malformed fields");
    }
    typed.ok_or("response contains malformed fields")
}

impl ResponseHead {
    pub(crate) fn pseudo(
        &mut self,
        name: &[u8],
        value: &[u8],
    ) -> std::result::Result<(), &'static str> {
        if self.regular || name != b":status" || self.status.is_some() {
            return Err("response contains malformed pseudo-headers");
        }
        let Ok(code) = StatusCode::from_bytes(value) else {
            return Err("response contains malformed :status pseudo-header");
        };
        if code == StatusCode::SWITCHING_PROTOCOLS {
            return Err("status 101 is forbidden");
        }
        self.status = Some(code);
        Ok(())
    }

    pub(crate) fn field(
        &mut self,
        field: std::result::Result<(HeaderName, HeaderValue), &'static str>,
    ) -> std::result::Result<(), &'static str> {
        self.regular = true;
        let (name, value) = field?;
        if name == http::header::CONTENT_LENGTH {
            if self.length.is_some() {
                return Err("response has multiple content-length fields");
            }
            self.length = Some(
                value
                    .to_str()
                    .ok()
                    .and_then(crate::util::content_length)
                    .ok_or("response has invalid content-length")?,
            );
        }
        self.headers.append(name, value);
        Ok(())
    }

    pub(crate) fn finish(
        self,
    ) -> std::result::Result<(StatusCode, HeaderMap, Option<u64>), &'static str> {
        self.status
            .map(|status| (status, self.headers, self.length))
            .ok_or("response missing :status pseudo-header")
    }
}

#[cfg(feature = "websocket")]
pub(crate) fn string_pairs(fields: &HeaderMap) -> Vec<(String, String)> {
    fields
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_owned(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect()
}

/// A response body as a transport returns it.
pub enum TransportBody {
    /// The whole body, read before return.
    Buffered(Vec<u8>),
    /// The body, streamed after the head.
    Streaming(Box<BodyStream>),
}

impl std::fmt::Debug for TransportBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Buffered(b) => f.debug_struct("Buffered").field("len", &b.len()).finish(),
            Self::Streaming(_) => f.debug_struct("Streaming").finish_non_exhaustive(),
        }
    }
}

/// A response as a transport returns it, before decoding and cookies.
#[derive(Debug)]
pub struct TransportResponse {
    /// The response status.
    pub status: StatusCode,
    /// The response header fields.
    pub headers: crate::core::response_head::Fields,
    /// The response trailer fields, when the response has any.
    pub trailers: Option<Box<HeaderMap>>,
    /// The response body.
    pub body: TransportBody,
    pub(crate) version: http::Version,
    pub(crate) tls: Option<crate::pool::TlsInfo>,
    pub(crate) timing: crate::core::ResponseTiming,
}

const _: () = assert!(
    size_of::<TransportResponse>() <= 160,
    "TransportResponse moves through every response channel; keep it at most 160 bytes"
);

pub(crate) struct Prepared<'a> {
    pub(crate) method: &'a str,
    pub(crate) url: &'a url::Url,
    pub(crate) headers: RequestHeaders,
    pub(crate) body: Body,
    pub(crate) proxy: Option<&'a crate::core::ProxyUrl>,
    pub(crate) response: ResponseMode,
}

impl<'a> Prepared<'a> {
    fn again(&self, body: Body) -> Self {
        Prepared {
            method: self.method,
            url: self.url,
            headers: self.headers.clone(),
            body,
            proxy: self.proxy,
            response: self.response,
        }
    }
}

pub(crate) async fn send_request_auto(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    req: Prepared<'_>,
) -> Result<TransportResponse> {
    let browser = H1Dial::Browser(h2_config);
    if req.url.scheme() == "http" {
        return send_h1(pool, connector, browser, req, None)
            .await?
            .response();
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
    let retry = req.body.replay().map(|body| req.again(body));
    match send_h2_on(pool, connector, h2_config, req, Some(opened)).await {
        Err(e) if is_h2_alpn_mismatch(&e) => {
            tracing::debug!(error = %e, "H2 ALPN mismatch, falling back to HTTP/1.1");
            let Some(retry) = retry else {
                return Err(Error::new(Kind::Request).with_message(
                    "the origin negotiated HTTP/1.1 and a streaming request body cannot be \
                     replayed; send the request again, buffer the body, or use \
                     Protocol::Http1",
                ));
            };
            Box::pin(send_h1_or_upgrade(pool, connector, h2_config, retry, None)).await
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
    let retry = req.again(Body::default());
    let browser = H1Dial::Browser(h2_config);
    match send_h1(pool, connector, browser, req, opened).await? {
        H1Outcome::Response(resp) => Ok(resp),
        H1Outcome::Upgraded { opened, body } => {
            if let Some(host) = retry.url.host_str() {
                let port = retry.url.port_or_known_default().unwrap_or(443);
                pool.forget(crate::pool::OriginFlag::H1Only, host, port, retry.proxy);
            }
            send_h2_on(
                pool,
                connector,
                h2_config,
                Prepared { body, ..retry },
                Some(opened),
            )
            .await
        }
    }
}

pub(crate) use crate::pool::send_request as send_h2_on;

pub(crate) struct H2Parts<'a> {
    pub(crate) host: &'a str,
    pub(crate) port: u16,
    pub(crate) pseudo: crate::h2::connection::PseudoHeaders,
    pub(crate) headers: RequestHeaders,
    pub(crate) body: Body,
    pub(crate) proxy: Option<&'a crate::core::ProxyUrl>,
    pub(crate) mode: ResponseMode,
}

pub(crate) fn h2_parts(req: Prepared<'_>) -> Result<H2Parts<'_>> {
    let Prepared {
        method,
        url,
        headers,
        body,
        proxy,
        response,
    } = req;
    let pseudo = h2::request_pseudo(method, url)?;
    let headers = if h2_ready(&headers) {
        headers
    } else {
        let mut headers = headers.into_vec();
        strip_connection_specific_headers(&mut headers)?;
        RequestHeaders::from(headers)
    };
    let (host, port) = crate::util::host_port(url)?;
    Ok(H2Parts {
        host,
        port,
        pseudo,
        headers,
        body,
        proxy,
        mode: response,
    })
}

pub(crate) fn check_framing(headers: &[HeaderPair]) -> Result<()> {
    let (mut te, mut cl) = (0, 0);
    for (name, _) in headers {
        match name.len() {
            17 if name.eq_ignore_ascii_case("transfer-encoding") => te += 1,
            14 if name.eq_ignore_ascii_case("content-length") => cl += 1,
            _ => {}
        }
    }
    if te > 1 {
        return Err(Error::config(format!(
            "{te} Transfer-Encoding headers on one request: framing would be ambiguous"
        )));
    }
    if te > 0 && cl > 0 {
        return Err(Error::config(
            "Transfer-Encoding together with Content-Length on one request: framing would be ambiguous",
        ));
    }
    Ok(())
}

pub(crate) fn h2_ready(headers: &[HeaderPair]) -> bool {
    check_framing(headers).is_ok()
        && headers.iter().all(|(name, value)| {
            !crate::util::has_ascii_uppercase(name) && sent_on(http::Version::HTTP_2, name, value)
        })
}

pub(crate) fn strip_connection_specific_headers(headers: &mut Vec<HeaderPair>) -> Result<()> {
    check_framing(headers)?;
    headers.retain_mut(|(name, value)| {
        if crate::util::has_ascii_uppercase(name) {
            *name = name.to_lowercase().into();
        }
        sent_on(http::Version::HTTP_2, name, value)
    });
    Ok(())
}

pub(crate) fn sent_on(version: http::Version, name: &str, value: &str) -> bool {
    if version == http::Version::HTTP_11 {
        return !name.eq_ignore_ascii_case("priority");
    }
    let connection_specific = match name.len() {
        2 => return !name.eq_ignore_ascii_case("te") || value.eq_ignore_ascii_case("trailers"),
        4 => "host",
        7 => "upgrade",
        10 if name.eq_ignore_ascii_case("keep-alive") => return false,
        10 => "connection",
        14 => "http2-settings",
        16 => "proxy-connection",
        17 => "transfer-encoding",
        _ => return true,
    };
    !name.eq_ignore_ascii_case(connection_specific)
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
        headers,
        body,
        response,
        proxy,
        ..
    } = req;
    let mut headers = headers.into_vec();
    let (host, port) = crate::util::host_port(url)?;
    let authority = crate::util::authority(url, host, port);
    let full_path = crate::util::request_target(url);

    strip_connection_specific_headers(&mut headers)?;

    crate::pool::send_request_h3_pooled(
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
    .await
}

pub(crate) fn is_h2_alpn_mismatch(err: &Error) -> bool {
    err.alpn()
        .is_some_and(|alpn| alpn.as_bytes() != crate::tls::alpn::H2)
}

#[cfg(test)]
mod alpn_fallback_tests;
#[cfg(test)]
mod h2_header_hygiene_tests;
