use std::sync::Arc;

use super::{Prepared, TransportBody, TransportResponse, adopt, check_framing, status};
use crate::core::body::{Body, BodyKind};
use crate::core::error::{Error, Kind, Result};
use crate::core::response::HttpVersion;
use crate::core::session::decompress::BodyLimit;
use crate::pool::{H1Body, H1PooledError, H1ResponseBody, H1Target, Pool};
use crate::tls::FingerprintConnector;

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
            let mut headers = headers;
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case("priority"));
            if !matches!(parsed.scheme(), "http" | "https") {
                (H1Target::OriginForm, headers)
            } else {
                if let Some(credentials) = crate::util::proxy_basic_auth(&parsed) {
                    headers.push((
                        "Proxy-Authorization".into(),
                        std::borrow::Cow::Owned(credentials),
                    ));
                }
                (H1Target::AbsoluteForm, headers)
            }
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

    Ok(TransportResponse {
        status: status(resp.status)?,
        headers: adopt(resp.headers),
        trailers: Vec::new(),
        body: transport_body,
        final_url: url.clone(),
        version: HttpVersion::Http1_1,
        tls: resp.tls,
        timing: resp.timing,
    })
}

pub(crate) fn h1_error_to_core(e: H1PooledError) -> Error {
    match e {
        H1PooledError::Config(m) => Error::new(Kind::Config).with_message(m),
        H1PooledError::Tls(error) => Error::from(error),
        H1PooledError::Io(io) => match BodyLimit::of_io(&io) {
            Some(limit) => limit.error(),
            None => Error::new(Kind::Io).with_source(io),
        },
        H1PooledError::Http(m) => Error::new(Kind::Request).with_message(m),
        H1PooledError::ConnectionClosed(ctx) => {
            Error::new(Kind::Io).with_source(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!("connection closed {ctx}"),
            ))
        }
    }
}
