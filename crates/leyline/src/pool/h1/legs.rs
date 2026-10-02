use std::sync::Arc;
use std::time::Instant;

use super::dial::{Dialed, H1Dial, NEGOTIATED_H2, open_new};
use super::{H1Body, H1PooledError, H1Response, checkout_live_h1};
use crate::ResponseTiming;
use crate::h2::client::H2Client;
use crate::pool::types::{Opened, PoolKey};
use crate::pool::{H1Slot, Pool, TlsInfo};
use crate::tls::FingerprintConnector;
use crate::{Error, Kind};

pub(crate) enum H1Outcome {
    Response(H1Response),
    Upgraded {
        opened: Opened<H2Client>,
        body: H1Body,
    },
}

impl H1Outcome {
    pub(crate) fn upgraded_error() -> H1PooledError {
        H1PooledError::NotResendable(
            Error::new(Kind::Http2)
                .with_message("the origin negotiated HTTP/2 on a new connection")
                .with_alpn(NEGOTIATED_H2),
        )
    }

    #[cfg(feature = "bench-internals")]
    pub(super) fn into_response(self) -> Result<H1Response, H1PooledError> {
        match self {
            Self::Response(resp) => Ok(resp),
            Self::Upgraded { .. } => Err(Self::upgraded_error()),
        }
    }
}

pub(super) struct FirstLegs {
    pub(super) started: Instant,
    pub(super) pooled: Option<(H1Slot, TlsInfo)>,
    pub(super) fresh: Option<Opened<H1Slot>>,
}

pub(super) fn first_legs(
    pool: &Arc<Pool>,
    key: &PoolKey,
    opened: Option<Opened<H1Slot>>,
) -> FirstLegs {
    match opened {
        Some(opened) if opened.connect_ms.is_some() => FirstLegs {
            started: opened.started,
            pooled: None,
            fresh: Some(opened),
        },
        Some(opened) => FirstLegs {
            started: opened.started,
            pooled: Some((opened.conn, opened.tls)),
            fresh: None,
        },
        None => FirstLegs {
            started: Instant::now(),
            pooled: checkout_live_h1(pool, key),
            fresh: None,
        },
    }
}

pub(super) enum Leg {
    H1(H1Slot, TlsInfo, u32),
    H2(Opened<H2Client>),
}

pub(super) async fn fresh_leg(
    pool: &Pool,
    key: &PoolKey,
    connector: &FingerprintConnector,
    dial: H1Dial<'_>,
    fresh: Option<Opened<H1Slot>>,
) -> Result<Leg, H1PooledError> {
    if let Some(opened) = fresh {
        let connect_ms = opened.connect_ms.unwrap_or_default();
        return Ok(Leg::H1(opened.conn, opened.tls, connect_ms));
    }
    let started = Instant::now();
    match open_new(pool, key, connector, dial).await? {
        Dialed::H1(io, tls) => Ok(Leg::H1(H1Slot { io }, tls, ResponseTiming::millis(started))),
        Dialed::H2(conn, tls) => Ok(Leg::H2(Opened::fresh((conn, tls), started))),
    }
}
