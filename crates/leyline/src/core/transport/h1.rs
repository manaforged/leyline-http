use std::future::Future;
use std::sync::Arc;

use futures_util::future::Either;

use super::{Prepared, check_framing};
use crate::core::body::Body;
use crate::core::error::Result;

use crate::core::request_headers::RequestHeaders;
use crate::pool::h1::{H1Dial, H1Outcome, H1Request, send_h1_pooled};
use crate::pool::{H1Slot, H1Target, Opened, Pool};
use crate::tls::FingerprintConnector;

pub(crate) fn send_h1<'a>(
    pool: &'a Arc<Pool>,
    connector: &'a FingerprintConnector,
    dial: H1Dial<'a>,
    req: Prepared<'a>,
    opened: Option<Opened<H1Slot>>,
) -> impl Future<Output = Result<H1Outcome>> + 'a {
    match h1_request(req) {
        Ok((request, body)) => {
            Either::Right(send_h1_pooled(pool, connector, request, body, opened, dial))
        }
        Err(e) => Either::Left(std::future::ready(Err(e))),
    }
}

fn h1_request(req: Prepared<'_>) -> Result<(H1Request<'_>, Body)> {
    let Prepared {
        method,
        url,
        headers,
        body,
        proxy,
        response,
    } = req;
    check_framing(&headers)?;
    let (host, port) = crate::util::host_port(url)?;
    let scheme = url.scheme();
    let (target, headers) = h1_target(scheme, proxy, headers);
    let request = H1Request {
        scheme,
        host,
        port,
        method,
        url,
        headers,
        proxy,
        target,
        response,
    };
    Ok((request, body))
}

fn h1_target(
    scheme: &str,
    proxy: Option<&crate::core::ProxyUrl>,
    mut headers: RequestHeaders,
) -> (H1Target, RequestHeaders) {
    let (target, credentials) = proxy_target(scheme, proxy);
    if target == H1Target::OriginForm && headers.h1_lines().is_some() {
        return (target, headers);
    }
    let pairs = headers.to_mut();
    pairs.retain(|(k, v)| super::sent_on(http::Version::HTTP_11, k, v));
    if let Some(credentials) = credentials {
        pairs.push((
            "Proxy-Authorization".into(),
            crate::header_str::HeaderStr::from(credentials),
        ));
    }
    (target, headers)
}

fn proxy_target(scheme: &str, proxy: Option<&crate::core::ProxyUrl>) -> (H1Target, Option<String>) {
    let Some(proxy_url) = proxy.filter(|_| scheme == "http") else {
        return (H1Target::OriginForm, None);
    };
    let parsed = proxy_url.url();
    if !matches!(parsed.scheme(), "http" | "https") {
        return (H1Target::OriginForm, None);
    }
    (
        H1Target::AbsoluteForm,
        crate::util::proxy_basic_auth(parsed),
    )
}
