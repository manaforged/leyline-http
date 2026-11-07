use std::ops::ControlFlow;
use std::sync::Arc;

use http::Method;

use super::{Attempt, LegTrace, Reopened, Sent, h2_failure, resending};
use crate::error::Error;
use crate::h2::client::{H2Client, H2ConnectStream};
use crate::h2::connection::PseudoHeaders;
use crate::h2::{H2Config, H2Error};
use crate::net::ProxyUrl;
use crate::pool::Pool;
use crate::pool::connect::{H2Leg, checkout_h2_opened, open_h2};
use crate::pool::liveness::confirm;
use crate::pool::types::{Opened, PoolKey};
use crate::profile::Carrier;
use crate::tls::FingerprintConnector;
use crate::wire::{Body, HeaderPair, Scheme};

struct Tunnel<'a> {
    pool: &'a Arc<Pool>,
    key: &'a PoolKey,
    pseudo: &'a PseudoHeaders,
    headers: &'a [HeaderPair],
}

/// Opens an RFC 8441 extended CONNECT stream on a pooled or new HTTP/2 connection, or returns `None` when the origin cannot carry one.
pub(crate) async fn open_extended_connect(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    url: &url::Url,
    proxy: Option<&ProxyUrl>,
    pseudo: PseudoHeaders,
    headers: Vec<HeaderPair>,
) -> Result<Option<H2ConnectStream>, Error> {
    let (host, port) = crate::wire::host_port(url)?;
    let mut key = pool.key(Scheme::Https, host, port, proxy, Carrier::Tcp);
    let opened = match checkout_h2_opened(
        pool,
        connector,
        h2_config,
        &mut key,
        std::time::Instant::now(),
    )
    .await?
    {
        Some(H2Leg::Mux(opened)) => opened,
        None | Some(H2Leg::Http1 { .. }) => return Ok(None),
    };
    key.lane = opened.lane;
    let key = &key;
    let ctx = &Tunnel {
        pool,
        key,
        pseudo: &pseudo,
        headers: &headers,
    };
    let trace = LegTrace {
        host,
        port,
        method: &Method::CONNECT,
        path: pseudo.path.as_str(),
        version: http::Version::HTTP_2,
    };
    resending(
        &trace,
        Some(opened),
        Body::default(),
        move |opened, body, first| open_on(ctx, opened, body, first),
        move |body| reopen(pool, connector, h2_config, key.clone(), body),
    )
    .await
}

async fn reopen(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    key: PoolKey,
    body: Body,
) -> Result<Reopened<H2Client, Option<H2ConnectStream>>, Error> {
    Ok(match open_h2(pool, connector, h2_config, key).await? {
        H2Leg::Mux(opened) => ControlFlow::Continue((opened, body)),
        H2Leg::Http1 { .. } => ControlFlow::Break(None),
    })
}

async fn open_on(
    ctx: &Tunnel<'_>,
    mut opened: Opened<H2Client>,
    body: Body,
    first: bool,
) -> Result<Sent<Option<H2ConnectStream>>, Error> {
    let leg = opened.leg();
    if let Some(probe) = opened.probe.take()
        && !confirm(ctx.pool, ctx.key, &opened.conn, *probe).await
    {
        return Ok(Sent::Retry(body));
    }
    match opened
        .conn
        .open_extended_connect(ctx.pseudo.clone(), ctx.headers.to_vec(), opened.slot)
        .await
    {
        Ok(stream) => Ok(Sent::Done(Some(stream))),
        Err(H2Error::NoExtendedConnect) => Ok(Sent::Done(None)),
        Err(e) => {
            let attempt = Attempt {
                leg,
                first,
                method: &Method::CONNECT,
                stale_resend: true,
            };
            ctx.pool
                .resend(
                    ctx.key,
                    &opened.conn,
                    h2_failure(&e),
                    attempt,
                    body.replay(),
                    e,
                )
                .map(Sent::Retry)
        }
    }
}
