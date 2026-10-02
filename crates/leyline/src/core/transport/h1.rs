use std::sync::Arc;

use super::{Prepared, TransportBody, TransportResponse, adopt, check_framing, status};
use crate::core::body::{Body, BodyKind};
use crate::core::error::{Error, Kind, Result};
use crate::core::response::HttpVersion;
use crate::core::session::decompress::BodyLimit;
use crate::h2::client::H2Client;
use crate::pool::h1::{H1Dial, H1Outcome, H1Request, h1err_to_io, send_h1_pooled};
use crate::pool::{H1Body, H1PooledError, H1ResponseBody, H1Slot, H1Target, Opened, Pool};
use crate::profile::preset::HeaderPair;
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

pub(crate) async fn send_request_h1(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    dial: H1Dial<'_>,
    req: Prepared<'_>,
) -> Result<TransportResponse> {
    match send_h1_on(pool, connector, dial, req, None).await? {
        H1Sent::Done(resp) => Ok(*resp),
        H1Sent::Upgraded(..) => Err(h1_error_to_core(H1Outcome::upgraded_error())),
    }
}

pub(super) enum H1Sent {
    Done(Box<TransportResponse>),
    Upgraded(Opened<H2Client>, Body),
}

fn h1_to_body(body: H1Body) -> Body {
    Body(match body {
        H1Body::Empty => BodyKind::Empty,
        H1Body::Buffered(b) => BodyKind::Bytes(b),
        H1Body::FixedStream { stream, length } => BodyKind::Stream {
            stream,
            length_hint: Some(length),
        },
        H1Body::ChunkedStream { stream } => BodyKind::Stream {
            stream,
            length_hint: None,
        },
    })
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
pub(super) async fn send_h1_on(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    dial: H1Dial<'_>,
    req: Prepared<'_>,
    opened: Option<Opened<H1Slot>>,
) -> Result<H1Sent> {
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

    let (target, headers) = h1_target(scheme, proxy, headers)?;

    let h1_body = body_to_h1(body);

    let outcome = send_h1_pooled(
        pool,
        connector,
        H1Request {
            scheme,
            host,
            port,
            method,
            url,
            headers: headers
                .into_iter()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect(),
            proxy,
            target,
            stream: stream_response,
            opened,
        },
        h1_body,
        dial,
    )
    .await
    .map_err(h1_error_to_core)?;
    let resp = match outcome {
        H1Outcome::Response(resp) => resp,
        H1Outcome::Upgraded { opened, body } => {
            return Ok(H1Sent::Upgraded(opened, h1_to_body(body)));
        }
    };

    let transport_body = match resp.body {
        H1ResponseBody::Buffered(b) => TransportBody::Buffered(b),
        H1ResponseBody::Streaming(s) => TransportBody::Streaming(s),
    };

    Ok(H1Sent::Done(Box::new(TransportResponse {
        status: status(resp.status)?,
        headers: adopt(resp.headers),
        trailers: Vec::new(),
        body: transport_body,
        final_url: url.clone(),
        version: HttpVersion::Http1_1,
        tls: resp.tls,
        timing: resp.timing,
    })))
}

pub(crate) fn h1_error_to_core(e: H1PooledError) -> Error {
    match e {
        H1PooledError::Config(m) => Error::new(Kind::Config).with_message(m),
        H1PooledError::Tls(error) => Error::from(error),
        H1PooledError::Io(io) => match BodyLimit::of_io(&io) {
            Some(limit) => limit.error(),
            None => Error::new(Kind::Io).with_source(io),
        },
        H1PooledError::RequestBody(io) => Error::from_request_body(io),
        H1PooledError::NotResendable(error) => error,
        other => Error::new(Kind::Io).with_source(h1err_to_io(other)),
    }
}

fn h1_target(
    scheme: &str,
    proxy: Option<&str>,
    mut headers: Vec<HeaderPair>,
) -> Result<(H1Target, Vec<HeaderPair>)> {
    headers.retain(|(k, v)| super::sent_on(HttpVersion::Http1_1, k, v));
    let Some(proxy_url) = proxy.filter(|_| scheme == "http") else {
        return Ok((H1Target::OriginForm, headers));
    };
    let parsed = url::Url::parse(proxy_url)
        .map_err(|e| Error::new(Kind::Config).with_message(format!("invalid proxy URL: {e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Ok((H1Target::OriginForm, headers));
    }
    if let Some(credentials) = crate::util::proxy_basic_auth(&parsed) {
        headers.push((
            "Proxy-Authorization".into(),
            std::borrow::Cow::Owned(credentials),
        ));
    }
    Ok((H1Target::AbsoluteForm, headers))
}
