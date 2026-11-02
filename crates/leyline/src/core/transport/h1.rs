use std::sync::Arc;

use super::{Prepared, check_framing};
use crate::core::error::Result;

use crate::pool::h1::{H1Dial, H1Outcome, H1Request, send_h1_pooled};
use crate::pool::{H1Slot, H1Target, Opened, Pool};
use crate::profile::preset::HeaderPair;
use crate::tls::FingerprintConnector;

pub(crate) async fn send_h1(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    dial: H1Dial<'_>,
    req: Prepared<'_>,
    opened: Option<Opened<H1Slot>>,
) -> Result<H1Outcome> {
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
    send_h1_pooled(pool, connector, request, body, opened, dial).await
}

fn h1_target(
    scheme: &str,
    proxy: Option<&crate::core::ProxyUrl>,
    mut headers: Vec<HeaderPair>,
) -> (H1Target, Vec<HeaderPair>) {
    headers.retain(|(k, v)| super::sent_on(http::Version::HTTP_11, k, v));
    let Some(proxy_url) = proxy.filter(|_| scheme == "http") else {
        return (H1Target::OriginForm, headers);
    };
    let parsed = proxy_url.url();
    if !matches!(parsed.scheme(), "http" | "https") {
        return (H1Target::OriginForm, headers);
    }
    if let Some(credentials) = crate::util::proxy_basic_auth(parsed) {
        headers.push((
            "Proxy-Authorization".into(),
            crate::header_str::HeaderStr::from(credentials),
        ));
    }
    (H1Target::AbsoluteForm, headers)
}
