use std::sync::Arc;

use super::Prepared;
use crate::error::{Error, Kind, Result};
use crate::h2::H2Config;
use crate::h2::client::H2Client;
use crate::tls::FingerprintConnector;
use crate::wire::{Body, Scheme, TransportResponse};

use super::Pool;
use super::connect::{Negotiated, negotiate};
use super::h1::{H1Outcome, send_h1, send_http1};
use super::send::{H2Outcome, send_request};
use super::types::{H1Slot, Opened};

/// The HTTP versions a request may use over TCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Route {
    Auto,
    Http1,
    Http2,
}

/// Sends one request over TCP on the versions `route` allows.
pub(crate) async fn send(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    route: Route,
    req: Prepared<'_>,
) -> Result<Box<TransportResponse>> {
    match route {
        Route::Auto => send_auto(pool, connector, h2_config, req).await,
        Route::Http1 => send_http1(pool, connector, req).await,
        Route::Http2 => send_request(pool, connector, h2_config, req, None)
            .await?
            .response(),
    }
}

async fn send_auto(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    req: Prepared<'_>,
) -> Result<Box<TransportResponse>> {
    if Scheme::of(req.url) == Some(Scheme::Http) {
        return send_http1(pool, connector, req).await;
    }
    let (host, port) = crate::wire::host_port(req.url)?;
    let host = host.to_string();
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
) -> Result<Box<TransportResponse>> {
    let retry = req.body.replay().map(|body| req.again(body));
    match send_request(pool, connector, h2_config, req, Some(opened)).await? {
        H2Outcome::Response(response) => Ok(response),
        H2Outcome::Http1(not_h2) => {
            tracing::debug!(error = %not_h2, "H2 ALPN mismatch, falling back to HTTP/1.1");
            let Some(retry) = retry else {
                return Err(Error::new(Kind::Request).with_message(
                    "the origin negotiated HTTP/1.1 and a streaming request body cannot be \
                     replayed; send the request again, buffer the body, or use \
                     Protocol::Http1",
                ));
            };
            Box::pin(send_h1_or_upgrade(pool, connector, h2_config, retry, None)).await
        }
    }
}

async fn send_h1_or_upgrade(
    pool: &Arc<Pool>,
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    req: Prepared<'_>,
    opened: Option<Opened<H1Slot>>,
) -> Result<Box<TransportResponse>> {
    let retry = req.again(Body::default());
    match send_h1(pool, connector, Some((h2_config, ())), req, opened).await? {
        H1Outcome::Response(resp) => Ok(resp),
        H1Outcome::Upgraded { opened, body, .. } => send_request(
            pool,
            connector,
            h2_config,
            Prepared { body, ..retry },
            Some(opened),
        )
        .await?
        .response(),
    }
}
