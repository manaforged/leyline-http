use std::io;
use std::sync::Arc;

use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};

use crate::h2::connection::{HeaderPair, PseudoHeaders};

use super::ResponseSink;

pub(crate) const MAX_FLOW_WINDOW: i64 = 0x7FFF_FFFF;

pub(crate) fn checked_window_add(current: i64, delta: i64) -> Result<i64, i64> {
    let new_win = current.saturating_add(delta);
    if new_win > MAX_FLOW_WINDOW {
        Err(new_win)
    } else {
        Ok(new_win)
    }
}

pub(crate) enum DriverRequestBody {
    None,
    Buffered(Bytes),
    Streaming(mpsc::Receiver<io::Result<Bytes>>),
}

pub struct Head {
    pub pseudo: PseudoHeaders,
    pub headers: Vec<HeaderPair>,
}

pub(crate) enum DriverCommand {
    SendRequest {
        head: Arc<Head>,
        body: DriverRequestBody,
        sink: ResponseSink,
    },
    OpenConnect {
        pseudo: PseudoHeaders,
        headers: Vec<crate::h2::connection::HeaderPair>,
        write_rx: mpsc::Receiver<io::Result<Bytes>>,
        sink: ResponseSink,
    },
    Ping {
        ack_tx: oneshot::Sender<()>,
    },
}

#[cfg(test)]
mod flow_control_tests;
