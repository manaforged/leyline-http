//! Transport layer — connects TLS/plain TCP and sends requests.

use std::sync::Arc;

use crate::h2::config::H2Config;
use crate::h2::connection::PseudoHeaders;
use crate::pool::{H1Body, H1PooledError, H1ResponseBody, H1Target, Pool};
use crate::profile::preset::HeaderPair;
use crate::tls::FingerprintConnector;
use crate::util::{base64_encode, percent_decode};
use bytes::Bytes;
use futures_util::StreamExt;

use crate::core::body::Body;
use crate::core::body_stream::BodyStream;
use crate::core::error::{Error, Result};
use crate::core::response::HttpVersion;

const MAX_H1_BODY_BYTES: usize = 100 * 1024 * 1024;

/// Body shape returned by a transport.
pub(crate) enum TransportBody {
    Buffered(Vec<u8>),
    Streaming(BodyStream),
}

/// Response returned by a transport.
pub(crate) struct TransportResponse {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(crate::core::HeaderStr, crate::core::HeaderStr)>,
    pub(crate) body: TransportBody,
    pub(crate) final_url: String,
    pub(crate) version: HttpVersion,
    pub(crate) tls_alpn: Option<String>,
    pub(crate) peer_cert_der: Option<Vec<u8>>,
    pub(crate) tls_version: Option<String>,
    pub(crate) tls_cipher: Option<String>,
    /// Wall-clock timing breakdown for this hop.
    pub(crate) timing: crate::core::ResponseTiming,
}

/// Send an HTTP request with browser-compatible defaults.
#[tracing::instrument(
    name = "transport.auto",
    level = "debug",
    skip_all,
    fields(
        http.method = method,
        http.scheme = url.scheme(),
        http.host = url.host_str().unwrap_or(""),
        proxied = proxy.is_some(),
    )
)]
#[expect(
    clippy::too_many_arguments,
    reason = "flat per-request wire fields across one internal call path"
)]
pub(crate) async fn send_request_auto(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    method: &str,
    url: &url::Url,
    headers: Vec<HeaderPair>,
    body: Body,
    proxy: Option<&str>,
    stream_response: bool,
    header_order: Option<&[String]>,
) -> Result<TransportResponse> {
    if url.scheme() == "http" {
        return Box::pin(send_request_h1(
            pool,
            connector,
            method,
            url,
            headers,
            body,
            proxy,
            stream_response,
        ))
        .await;
    }

    let (h2_body, fallback_buf): (Body, Option<Bytes>) = if body.is_stream() {
        let buf = Box::pin(materialise_stream_body(body)).await?;
        (Body::from(buf.clone()), Some(buf))
    } else {
        match body {
            Body::Empty => (Body::Empty, None),
            Body::Bytes(b) => (Body::Bytes(b.clone()), Some(b)),
            Body::Stream { .. } => unreachable!("stream branch handled above"),
        }
    };

    match send_request_h2(
        pool,
        connector,
        h2_config,
        method,
        url,
        headers.clone(),
        h2_body,
        proxy,
        stream_response,
        header_order,
    )
    .await
    {
        Ok(resp) => Ok(resp),
        Err(e) if is_h2_alpn_mismatch(&e) => {
            tracing::debug!(error = %e, "H2 ALPN mismatch, falling back to HTTP/1.1");
            let fallback_body = match fallback_buf {
                Some(buf) => Body::from(buf),
                None => Body::Empty,
            };
            Box::pin(send_request_h1(
                pool,
                connector,
                method,
                url,
                headers,
                fallback_body,
                proxy,
                stream_response,
            ))
            .await
        }
        Err(e) => Err(e),
    }
}

/// Drain a streaming body into a single `Bytes` buffer.
async fn materialise_stream_body(body: Body) -> Result<Bytes> {
    match body {
        Body::Empty => Ok(Bytes::new()),
        Body::Bytes(b) => Ok(b),
        Body::Stream { mut stream, .. } => {
            let mut buf: Vec<u8> = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(Error::Io)?;
                buf.extend_from_slice(&chunk);
                if buf.len() > MAX_H1_BODY_BYTES {
                    return Err(Error::Body(format!(
                        "streaming request body exceeded {MAX_H1_BODY_BYTES} bytes"
                    )));
                }
            }
            Ok(Bytes::from(buf))
        }
    }
}

/// Send an HTTP request, reusing pooled H2 connections when available.
#[tracing::instrument(
    name = "transport.h2",
    level = "debug",
    skip_all,
    fields(http.method = method, http.host = url.host_str().unwrap_or(""))
)]
#[expect(
    clippy::too_many_arguments,
    reason = "flat per-request wire fields across one internal call path"
)]
pub(crate) async fn send_request_h2(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    method: &str,
    url: &url::Url,
    mut headers: Vec<HeaderPair>,
    body: Body,
    proxy: Option<&str>,
    stream_response: bool,
    header_order: Option<&[String]>,
) -> Result<TransportResponse> {
    if url.scheme() != "https" {
        return Err(Error::Config("HTTP/2 requires an https:// URL".into()));
    }

    let host = url
        .host_str()
        .ok_or_else(|| Error::Config("no host in URL".into()))?;
    let port = url.port_or_known_default().unwrap_or(443);

    let path = url.path();
    let query = url.query().map(|q| format!("?{q}")).unwrap_or_default();
    let pseudo = PseudoHeaders {
        method: method.to_string(),
        scheme: url.scheme().to_string(),
        authority: {
            let is_default_port =
                (url.scheme() == "https" && port == 443) || (url.scheme() == "http" && port == 80);
            if is_default_port {
                host.to_string()
            } else {
                format!("{host}:{port}")
            }
        },
        path: format!("{path}{query}"),
        protocol: None,
    };

    let h2_req_body = body_to_h2_request(body);

    strip_connection_specific_headers(&mut headers)?;

    if let Some(order) = header_order {
        crate::core::session::execute::reorder_headers(&mut headers, order);
    }

    let (resp, tls, timing) = crate::pool::send_request(
        pool,
        connector,
        h2_config,
        pseudo,
        headers,
        h2_req_body,
        proxy,
        stream_response,
    )
    .await?;

    let transport_body = match resp.body {
        crate::h2::client::ResponseBody::Buffered(b) => TransportBody::Buffered(b),
        crate::h2::client::ResponseBody::Streaming(rx) => {
            TransportBody::Streaming(BodyStream::new(rx))
        }
    };

    Ok(TransportResponse {
        status: resp.status,
        headers: resp.headers,
        body: transport_body,
        final_url: url.to_string(),
        version: HttpVersion::Http2,
        tls_alpn: Some("h2".to_string()),
        peer_cert_der: tls.peer_cert_der,
        tls_version: tls.version,
        tls_cipher: tls.cipher,
        timing,
    })
}

/// Translate a [`Body`] into the h2-crate request body shape.
fn body_to_h2_request(body: Body) -> crate::h2::client::RequestBody {
    match body {
        Body::Empty => crate::h2::client::RequestBody::None,
        Body::Bytes(b) => crate::h2::client::RequestBody::Buffered(b),
        Body::Stream {
            stream,
            length_hint,
        } => crate::h2::client::RequestBody::Streaming {
            stream,
            length_hint,
        },
    }
}

/// RFC 9113 §8.2.2: connection-specific headers must never be emitted on an H2 connection — a compliant peer rejects the stream.
pub(crate) fn strip_connection_specific_headers(headers: &mut Vec<HeaderPair>) -> Result<()> {
    let mut te = 0usize;
    let mut cl = 0usize;
    headers.retain_mut(|(name, value)| {
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "transfer-encoding" => {
                te += 1;
                false
            }
            "content-length" => {
                cl += 1;
                true
            }
            "connection" | "keep-alive" | "proxy-connection" | "upgrade" | "http2-settings" => {
                false
            }
            "te" => value.eq_ignore_ascii_case("trailers"),
            _ => true,
        }
    });
    if te > 1 {
        return Err(Error::Config(format!(
            "{te} Transfer-Encoding headers on one request: framing would be ambiguous"
        )));
    }
    if te > 0 && cl > 0 {
        return Err(Error::Config(
            "Transfer-Encoding together with Content-Length on one request: framing would be ambiguous"
                .into(),
        ));
    }
    for (name, _) in headers.iter_mut() {
        if name.chars().any(|c| c.is_ascii_uppercase()) {
            *name = name.to_lowercase().into();
        }
    }
    Ok(())
}

/// Translate a [`Body`] into the H1 pool's request body shape.
fn body_to_h1(body: Body) -> H1Body {
    match body {
        Body::Empty => H1Body::Empty,
        Body::Bytes(b) => H1Body::Buffered(b),
        Body::Stream {
            stream,
            length_hint: Some(length),
        } => H1Body::FixedStream { stream, length },
        Body::Stream {
            stream,
            length_hint: None,
        } => H1Body::ChunkedStream { stream },
    }
}

/// Send an HTTP/1.1 request through the HTTP/1.1 keep-alive pool.
#[tracing::instrument(
    name = "transport.h1",
    level = "debug",
    skip_all,
    fields(
        http.method = method,
        http.scheme = url.scheme(),
        http.host = url.host_str().unwrap_or(""),
    )
)]
#[expect(
    clippy::too_many_arguments,
    reason = "flat per-request wire fields across one internal call path"
)]
pub(crate) async fn send_request_h1(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    method: &str,
    url: &url::Url,
    headers: Vec<HeaderPair>,
    body: Body,
    proxy: Option<&str>,
    stream_response: bool,
) -> Result<TransportResponse> {
    let host = url
        .host_str()
        .ok_or_else(|| Error::Config("no host in URL".into()))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| Error::Config(format!("no default port for scheme {}", url.scheme())))?;
    let scheme = url.scheme();

    let (target, headers) = match (scheme, proxy) {
        ("http", Some(proxy_url)) => {
            let parsed = url::Url::parse(proxy_url)
                .map_err(|e| Error::Config(format!("invalid proxy URL: {e}")))?;
            if parsed.scheme() != "http" {
                return Err(Error::Config(
                    "plaintext HTTP currently supports http:// proxies only".into(),
                ));
            }
            let mut headers = headers;
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case("priority"));
            if let Some(password) = parsed.password() {
                let credentials = base64_encode(&format!(
                    "{}:{}",
                    percent_decode(parsed.username()),
                    percent_decode(password)
                ));
                headers.push((
                    "Proxy-Authorization".into(),
                    std::borrow::Cow::Owned(format!("Basic {credentials}")),
                ));
            }
            (H1Target::AbsoluteForm, headers)
        }
        _ => {
            let mut headers = headers;
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case("priority"));
            (H1Target::OriginForm, headers)
        }
    };

    let h1_body = body_to_h1(body);

    let resp = crate::pool::send_request_h1_pooled(
        pool,
        connector,
        scheme,
        host,
        port,
        method,
        url,
        headers
            .into_iter()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect(),
        h1_body,
        proxy,
        target,
        stream_response,
    )
    .await
    .map_err(h1_error_to_core)?;

    let transport_body = match resp.body {
        H1ResponseBody::Buffered(b) => TransportBody::Buffered(b),
        H1ResponseBody::Streaming(s) => TransportBody::Streaming(s),
    };

    let (tls_alpn, peer_cert_der, tls_version, tls_cipher) = match resp.tls {
        Some(info) => (
            Some("http/1.1".to_string()),
            info.peer_cert_der,
            info.version,
            info.cipher,
        ),
        None => (None, None, None, None),
    };

    Ok(TransportResponse {
        status: resp.status,
        headers: resp
            .headers
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect(),
        body: transport_body,
        final_url: url.to_string(),
        version: HttpVersion::Http1_1,
        tls_alpn,
        peer_cert_der,
        tls_version,
        tls_cipher,
        timing: crate::core::ResponseTiming::default(),
    })
}

fn h1_error_to_core(e: H1PooledError) -> Error {
    match e {
        H1PooledError::Config(m) => Error::Config(m),
        H1PooledError::Tls(error) => Error::Tls(error),
        H1PooledError::Io(io) => Error::Io(io),
        H1PooledError::Http(m) => Error::Http(m),
        H1PooledError::ConnectionClosed(ctx) => Error::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            format!("connection closed {ctx}"),
        )),
    }
}

/// Send an HTTP/3 request over QUIC.
#[cfg(feature = "http3")]
#[tracing::instrument(
    name = "transport.h3",
    level = "debug",
    skip_all,
    fields(http.method = method, http.host = url.host_str().unwrap_or(""))
)]
#[expect(
    clippy::too_many_arguments,
    reason = "flat per-request wire fields across one internal call path"
)]
pub(crate) async fn send_request_h3(
    pool: &Arc<Pool>,
    h3_config: &crate::quic::H3Config,
    profile: &crate::profile::BrowserProfile,
    trust: &crate::tls::TlsTrustConfig,
    method: &str,
    url: &url::Url,
    mut headers: Vec<HeaderPair>,
    body: Body,
    stream_response: bool,
) -> Result<TransportResponse> {
    let host = url
        .host_str()
        .ok_or_else(|| Error::Config("no host in URL".into()))?;
    let port = url.port_or_known_default().unwrap_or(443);
    let authority = if url.scheme() == "https" && port == 443 {
        host.to_string()
    } else {
        format!("{host}:{port}")
    };
    let path = url.path();
    let query = url.query().map(|q| format!("?{q}")).unwrap_or_default();
    let full_path = format!("{path}{query}");

    strip_connection_specific_headers(&mut headers)?;

    let (body_bytes, body_stream) = match body {
        Body::Empty => (None, None),
        Body::Bytes(b) => (Some(b), None),
        Body::Stream { stream, .. } => (None, Some(stream)),
    };

    let (resp, tls) = crate::pool::send_request_h3_pooled(
        pool,
        h3_config,
        profile,
        trust,
        host,
        port,
        method,
        &authority,
        &full_path,
        &headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<Vec<_>>(),
        body_bytes,
        body_stream,
        stream_response,
    )
    .await?;

    let transport_body = match resp.body {
        crate::quic::H3RespBody::Buffered(b) => TransportBody::Buffered(b),
        crate::quic::H3RespBody::Streaming(rx) => TransportBody::Streaming(BodyStream::new(rx)),
    };

    Ok(TransportResponse {
        status: resp.status,
        headers: resp
            .headers
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect(),
        body: transport_body,
        final_url: url.to_string(),
        version: HttpVersion::Http3,
        tls_alpn: Some("h3".to_string()),
        peer_cert_der: tls.peer_cert_der,
        tls_version: tls.version,
        tls_cipher: tls.cipher,
        timing: crate::core::ResponseTiming::default(),
    })
}

/// True when the H2 attempt failed because the server declined the `h2` ALPN (e.g. some CDN/WAF edges serve a cookieless interstitial over HTTP/1.1, replying with no ALPN).
fn is_h2_alpn_mismatch(err: &Error) -> bool {
    matches!(err, Error::AlpnMismatch { .. })
}

#[cfg(test)]
mod alpn_fallback_tests;
#[cfg(test)]
mod h2_header_hygiene_tests;
