//! Flow-control constants, the command protocol, and the driver task handle.

use std::io;

use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::h2::connection::{H2Response, PseudoHeaders};
use crate::h2::error::{ErrorCode, H2Error};

use super::super::types::H2ResponseEx;

/// Maximum permitted HTTP/2 flow-control window value (RFC 9113 §6.9.1).
pub(crate) const MAX_FLOW_WINDOW: i64 = 0x7FFF_FFFF;

/// Compute `current + delta` with the RFC 9113 §6.9.1 ceiling (2^31 − 1) applied.
pub(crate) fn checked_window_add(current: i64, delta: i64) -> Result<i64, i64> {
    let new_win = current.saturating_add(delta);
    if new_win > MAX_FLOW_WINDOW {
        Err(new_win)
    } else {
        Ok(new_win)
    }
}

/// Driver task handle.
pub struct DriverTask {
    pub(super) join: JoinHandle<Result<(), H2Error>>,
}

impl DriverTask {
    /// Wait for the driver to finish and return its final status.
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

    /// Abort the driver task forcibly.
    pub fn abort(self) {
        self.join.abort();
    }
}

/// Request body shape as seen by the driver.
pub(crate) enum DriverRequestBody {
    None,
    Buffered(Bytes),
    Streaming {
        rx: mpsc::Receiver<io::Result<Bytes>>,
        /// Retained for future use — H2 doesn't need content-length at the frame layer, but downstream consumers may want to know the declared size.
        #[expect(
            dead_code,
            reason = "retained for downstream consumers of the declared frame size; not read at the frame layer"
        )]
        length_hint: Option<u64>,
    },
}

/// Commands the driver accepts from handles.
pub(crate) enum DriverCommand {
    SendRequest {
        pseudo: PseudoHeaders,
        headers: Vec<crate::h2::connection::HeaderPair>,
        body: Option<Bytes>,
        /// Empty = no trailers.
        trailers: Vec<(String, String)>,
        response_tx: oneshot::Sender<Result<H2Response, H2Error>>,
    },
    SendRequestEx {
        pseudo: PseudoHeaders,
        headers: Vec<crate::h2::connection::HeaderPair>,
        body: DriverRequestBody,
        stream_response: bool,
        response_tx: oneshot::Sender<Result<H2ResponseEx, H2Error>>,
        /// For streaming-response mode, the sender side of the body chunk channel.
        stream_body_tx: mpsc::Sender<io::Result<Bytes>>,
    },
    /// Open an RFC 8441 extended CONNECT stream that stays bidirectional until the caller drops the [`H2ConnectStream`](crate::h2::client::connect_stream::H2ConnectStream) handle or the peer tears it down.
    OpenConnect {
        pseudo: PseudoHeaders,
        headers: Vec<crate::h2::connection::HeaderPair>,
        /// Outbound (user-side) data: a relay task forwards chunks read from here into the driver's internal body-chunk channel, which drives DATA-frame emission.
        write_rx: mpsc::Receiver<io::Result<Bytes>>,
        /// Delivered once :status HEADERS arrive — callers observe the WebSocket handshake outcome synchronously before receiving the stream handle.
        headers_tx: oneshot::Sender<Result<H2ResponseEx, H2Error>>,
        /// Inbound DATA chunks — driver forwards here, caller reads them through the [`H2ConnectStream`](crate::h2::client::connect_stream::H2ConnectStream) `AsyncRead` impl.
        body_tx: mpsc::Sender<io::Result<Bytes>>,
    },
}

#[cfg(test)]
mod flow_control_tests;
