use std::sync::Arc;

use bytes::Bytes;
use http::{HeaderName, HeaderValue, StatusCode};

use crate::h2::config::H2Config;
use crate::h2::connection::PseudoHeaders;
use crate::header_str::HeaderStr;
use crate::pool::{H1Body, H1PooledError, H1ResponseBody, H1Target, Pool};
use crate::profile::preset::HeaderPair;
use crate::tls::FingerprintConnector;
use crate::util::{base64_encode, percent_decode};

use crate::core::body::{Body, BodyKind};
use crate::core::body_stream::BodyStream;
use crate::core::error::{Error, Kind, Result};
use crate::core::response::HttpVersion;

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

pub(crate) enum TransportBody {
    Buffered(Vec<u8>),
    Streaming(BodyStream),
}

pub(crate) struct TransportResponse {
    pub(crate) status: StatusCode,
    pub(crate) headers: Vec<(HeaderName, HeaderValue)>,
    pub(crate) trailers: Vec<(HeaderName, HeaderValue)>,
    pub(crate) body: TransportBody,
    pub(crate) final_url: String,
    pub(crate) version: HttpVersion,
    pub(crate) tls_alpn: Option<HeaderStr>,
    pub(crate) peer_cert_der: Option<Vec<u8>>,
    pub(crate) tls_version: Option<String>,
    pub(crate) tls_cipher: Option<String>,
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

    let (h2_body, replay): (Body, Option<Body>) = match body.0 {
        BodyKind::Empty => (Body::default(), Some(Body::default())),
        BodyKind::Bytes(b) => (Body::bytes(b.clone()), Some(Body::bytes(b))),
        kind @ BodyKind::Stream { .. } => (Body(kind), None),
    };

    match send_request_h2(
        pool,
        connector,
        h2_config,
        Prepared {
            method,
            url,
            headers: headers.clone(),
            body: h2_body,
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
    if url.scheme() != "https" {
        return Err(Error::new(Kind::Config).with_message("HTTP/2 requires an https:// URL"));
    }

    let host = url
        .host_str()
        .ok_or_else(|| Error::new(Kind::Config).with_message("no host in URL"))?;
    let port = url.port_or_known_default().unwrap_or(443);

    let path = url.path();
    let query = url.query();
    let pseudo = PseudoHeaders {
        method: match method {
            "GET" => HeaderStr::from_static("GET"),
            "HEAD" => HeaderStr::from_static("HEAD"),
            "POST" => HeaderStr::from_static("POST"),
            "PUT" => HeaderStr::from_static("PUT"),
            "DELETE" => HeaderStr::from_static("DELETE"),
            "OPTIONS" => HeaderStr::from_static("OPTIONS"),
            "PATCH" => HeaderStr::from_static("PATCH"),
            "TRACE" => HeaderStr::from_static("TRACE"),
            "CONNECT" => HeaderStr::from_static("CONNECT"),
            m => HeaderStr::from(m),
        },
        scheme: HeaderStr::from_static("https"),
        authority: if port == 443 {
            HeaderStr::from(host)
        } else {
            HeaderStr::from(format!("{host}:{port}"))
        },
        path: HeaderStr::from(match query {
            Some(q) => {
                let mut target = String::with_capacity(path.len() + q.len() + 1);
                target.push_str(path);
                target.push('?');
                target.push_str(q);
                target
            }
            None => path.to_owned(),
        }),
        protocol: None,
    };

    let h2_req_body = body_to_h2_request(body);

    strip_connection_specific_headers(&mut headers)?;

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
        status: status(resp.status)?,
        headers: adopt(resp.headers),
        trailers: adopt(resp.trailers.unwrap_or_default()),
        body: transport_body,
        final_url: url.as_str().to_owned(),
        version: HttpVersion::Http2,
        tls_alpn: Some(HeaderStr::from_static("h2")),
        peer_cert_der: tls.peer_cert_der,
        tls_version: tls.version,
        tls_cipher: tls.cipher,
        timing,
    })
}

fn body_to_h2_request(body: Body) -> crate::h2::client::RequestBody {
    match body.0 {
        BodyKind::Empty => crate::h2::client::RequestBody::None,
        BodyKind::Bytes(b) => crate::h2::client::RequestBody::Buffered(b),
        BodyKind::Stream {
            stream,
            length_hint,
        } => crate::h2::client::RequestBody::Streaming {
            stream,
            length_hint,
        },
    }
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

fn body_to_h1(body: Body) -> H1Body {
    match body.0 {
        BodyKind::Empty => H1Body::Empty,
        BodyKind::Bytes(b) => H1Body::Buffered(b),
        BodyKind::Stream {
            stream,
            length_hint: Some(length),
        } => H1Body::FixedStream { stream, length },
        BodyKind::Stream {
            stream,
            length_hint: None,
        } => H1Body::ChunkedStream { stream },
    }
}

#[tracing::instrument(
    name = "transport.h1",
    level = "debug",
    skip_all,
    fields(
        http.method = req.method,
        http.scheme = req.url.scheme(),
        http.host = req.url.host_str().unwrap_or(""),
    )
)]
pub(crate) async fn send_request_h1(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    req: Prepared<'_>,
) -> Result<TransportResponse> {
    let Prepared {
        method,
        url,
        headers,
        body,
        proxy,
        stream_response,
    } = req;
    check_framing(&headers)?;
    let host = url
        .host_str()
        .ok_or_else(|| Error::new(Kind::Config).with_message("no host in URL"))?;
    let port = url.port_or_known_default().ok_or_else(|| {
        Error::new(Kind::Config)
            .with_message(format!("no default port for scheme {}", url.scheme()))
    })?;
    let scheme = url.scheme();

    let (target, headers) = match (scheme, proxy) {
        ("http", Some(proxy_url)) => {
            let parsed = url::Url::parse(proxy_url).map_err(|e| {
                Error::new(Kind::Config).with_message(format!("invalid proxy URL: {e}"))
            })?;
            if parsed.scheme() != "http" {
                return Err(Error::new(Kind::Config)
                    .with_message("plaintext HTTP currently supports http:// proxies only"));
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
            Some(HeaderStr::from_static("http/1.1")),
            info.peer_cert_der,
            info.version,
            info.cipher,
        ),
        None => (None, None, None, None),
    };

    Ok(TransportResponse {
        status: status(resp.status)?,
        headers: adopt(resp.headers),
        trailers: Vec::new(),
        body: transport_body,
        final_url: url.as_str().to_owned(),
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
        H1PooledError::Config(m) => Error::new(Kind::Config).with_message(m),
        H1PooledError::Tls(error) => Error::new(Kind::Tls).with_source(error),
        H1PooledError::Io(io) => Error::new(Kind::Io).with_source(io),
        H1PooledError::Http(m) => Error::new(Kind::Request).with_message(m),
        H1PooledError::ConnectionClosed(ctx) => {
            Error::new(Kind::Io).with_source(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!("connection closed {ctx}"),
            ))
        }
    }
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
    h3_config: &crate::quic::H3Config,
    profile: &crate::profile::BrowserProfile,
    trust: &crate::tls::TlsTrustConfig,
    req: Prepared<'_>,
) -> Result<TransportResponse> {
    let Prepared {
        method,
        url,
        mut headers,
        body,
        stream_response,
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
    let path = url.path();
    let query = url.query().map(|q| format!("?{q}")).unwrap_or_default();
    let full_path = format!("{path}{query}");

    strip_connection_specific_headers(&mut headers)?;

    let (body_bytes, body_stream) = match body.0 {
        BodyKind::Empty => (None, None),
        BodyKind::Bytes(b) => (Some(b), None),
        BodyKind::Stream { stream, .. } => (None, Some(stream)),
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
        status: status(resp.status)?,
        headers: adopt(resp.headers),
        trailers: adopt(resp.trailers),
        body: transport_body,
        final_url: url.as_str().to_owned(),
        version: HttpVersion::Http3,
        tls_alpn: Some(HeaderStr::from_static("h3")),
        peer_cert_der: tls.peer_cert_der,
        tls_version: tls.version,
        tls_cipher: tls.cipher,
        timing: crate::core::ResponseTiming::default(),
    })
}

fn is_h2_alpn_mismatch(err: &Error) -> bool {
    err.alpn().is_some()
}

#[cfg(test)]
mod alpn_fallback_tests;
#[cfg(test)]
mod h2_header_hygiene_tests;
