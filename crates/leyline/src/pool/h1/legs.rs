use std::sync::Arc;
use std::time::Instant;

use super::checkout_live_h1;
use super::streaming::Delivered;
use crate::h2::client::H2Client;
use crate::pool::types::{Opened, PoolKey};
use crate::pool::{H1Slot, Pool};
use crate::wire::Body;
use crate::wire::TransportResponse;

pub(crate) enum H1Outcome<W> {
    Response(Box<TransportResponse>),
    Upgraded {
        opened: Opened<H2Client>,
        body: Body,
        _offered: W,
    },
}

pub(super) struct Upgrade<W> {
    pub(super) opened: Opened<H2Client>,
    pub(super) body: Body,
    pub(super) offered: W,
}

pub(super) enum Reply<W> {
    Answered(Delivered),
    Upgraded(Box<Upgrade<W>>),
}

pub(super) fn first_leg(
    pool: &Arc<Pool>,
    key: &PoolKey,
    opened: Option<Opened<H1Slot>>,
    strict: bool,
    now: Instant,
) -> (Instant, Option<Opened<H1Slot>>) {
    match opened {
        Some(opened) => (opened.started, Some(opened)),
        None => (
            now,
            checkout_live_h1(pool, key, strict, now).map(|hit| Opened {
                started: now,
                ..Opened::pooled(hit)
            }),
        ),
    }
}
