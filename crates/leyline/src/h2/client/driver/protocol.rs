use std::io;
use std::sync::Arc;

use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::h2::connection::{H2Response, HeaderPair, PseudoHeaders};
use crate::h2::error::{ErrorCode, H2Error};

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

pub struct DriverTask {
    pub(super) join: JoinHandle<Result<(), H2Error>>,
}

impl DriverTask {
    pub async fn join(self) -> Result<(), H2Error> {
        match self.join.await {
            Ok(res) => res,
            Err(e) if e.is_cancelled() => Ok(()),
            Err(e) => Err(H2Error::Connection {
                code: ErrorCode::InternalError,
                reason: format!("driver task panicked: {e}"),
            }),
        }
    }

    pub fn abort(self) {
        self.join.abort();
    }
}

pub(crate) enum DriverRequestBody {
    None,
    Buffered(Bytes),
    Streaming {
        rx: mpsc::Receiver<io::Result<Bytes>>,
        #[expect(
            dead_code,
            reason = "retained for downstream consumers of the declared frame size; not read at the frame layer"
        )]
        length_hint: Option<u64>,
    },
}

pub(crate) struct Head {
    pub(crate) pseudo: PseudoHeaders,
    pub(crate) headers: Vec<HeaderPair>,
}

pub(crate) enum DriverCommand {
    SendRequest {
        pseudo: PseudoHeaders,
        headers: Vec<crate::h2::connection::HeaderPair>,
        body: Option<Bytes>,
        trailers: Vec<(String, String)>,
        response_tx: oneshot::Sender<Result<H2Response, H2Error>>,
    },
    SendRequestEx {
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
