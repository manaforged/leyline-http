#[cfg(feature = "websocket")]
use std::io;
use std::sync::Arc;

use bytes::Bytes;
#[cfg(feature = "websocket")]
use tokio::sync::mpsc;
use tokio::sync::oneshot;

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
    Streaming(crate::util::upload::BodyStream),
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
    #[cfg(feature = "websocket")]
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

impl DriverCommand {
    pub(crate) fn is_cancelled(&self) -> bool {
        match self {
            DriverCommand::SendRequest { sink, .. } => sink.is_cancelled(),
            #[cfg(feature = "websocket")]
            DriverCommand::OpenConnect { sink, .. } => sink.is_cancelled(),
            DriverCommand::Ping { ack_tx } => ack_tx.is_closed(),
        }
    }

    pub(crate) fn into_sink(self) -> Option<ResponseSink> {
        match self {
            DriverCommand::SendRequest { sink, .. } => Some(sink),
            #[cfg(feature = "websocket")]
            DriverCommand::OpenConnect { sink, .. } => Some(sink),
            DriverCommand::Ping { .. } => None,
        }
    }
}

#[cfg(test)]
mod flow_control_tests;
