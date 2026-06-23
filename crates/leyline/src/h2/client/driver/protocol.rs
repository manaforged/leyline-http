//! Flow-control constants, the command protocol, and the driver task handle.

use std::io;

use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::h2::connection::{H2Response, PseudoHeaders};
use crate::h2::error::{ErrorCode, H2Error};

use super::super::types::H2ResponseEx;

/// Maximum permitted HTTP/2 flow-control window value (RFC 9113
/// §6.9.1). A peer that pushes any window past this via WINDOW_UPDATE
/// or SETTINGS_INITIAL_WINDOW_SIZE is committing a FLOW_CONTROL_ERROR
/// and must be rejected to stay spec-conformant.
pub(crate) const MAX_FLOW_WINDOW: i64 = 0x7FFF_FFFF;

/// Compute `current + delta` with the RFC 9113 §6.9.1 ceiling
/// (2^31 − 1) applied. `delta` may be negative when a SETTINGS
/// frame *lowers* SETTINGS_INITIAL_WINDOW_SIZE — the spec allows
/// the resulting window to go negative but never above the cap.
///
/// Returns `Ok(new_window)` on success or `Err(new_window)` where
/// the error variant carries the post-add value so the caller can
/// include it in a diagnostic.
pub(crate) fn checked_window_add(current: i64, delta: i64) -> Result<i64, i64> {
    let new_win = current.saturating_add(delta);
    if new_win > MAX_FLOW_WINDOW {
        Err(new_win)
    } else {
        Ok(new_win)
    }
}

/// Driver task handle. Dropping it does **not** stop the driver — drop
/// all [`crate::h2::H2Client`] handles for graceful shutdown. This handle
/// exists so callers can `await` the driver's final status or force-abort it.
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

    /// Abort the driver task forcibly. Prefer dropping all handles.
    pub fn abort(self) {
        self.join.abort();
    }
}

/// Request body shape as seen by the driver. The user-provided `Stream`
/// is converted into an mpsc receiver before the command is enqueued.
pub(crate) enum DriverRequestBody {
    None,
    Buffered(Bytes),
    Streaming {
        rx: mpsc::Receiver<io::Result<Bytes>>,
        /// Retained for future use — H2 doesn't need content-length at
        /// the frame layer, but downstream consumers may want to know
        /// the declared size.
        #[allow(dead_code)]
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
        /// For streaming-response mode, the sender side of the body
        /// chunk channel. Ignored in buffered-response mode. The
        /// caller keeps the receiver and stitches it into the returned
        /// `H2ResponseEx` after receiving headers.
        stream_body_tx: mpsc::Sender<io::Result<Bytes>>,
    },
    /// Open an RFC 8441 extended CONNECT stream that stays
    /// bidirectional until the caller drops the
    /// [`H2ConnectStream`] handle or the peer tears it down. The
    /// request HEADERS carries no END_STREAM flag, so DATA frames
    /// flow in both directions for the lifetime of the stream.
    OpenConnect {
        pseudo: PseudoHeaders,
        headers: Vec<crate::h2::connection::HeaderPair>,
        /// Outbound (user-side) data: a relay task forwards chunks
        /// read from here into the driver's internal body-chunk
        /// channel, which drives DATA-frame emission.
        write_rx: mpsc::Receiver<io::Result<Bytes>>,
        /// Delivered once :status HEADERS arrive — callers observe
        /// the WebSocket handshake outcome synchronously before
        /// receiving the stream handle.
        headers_tx: oneshot::Sender<Result<H2ResponseEx, H2Error>>,
        /// Inbound DATA chunks — driver forwards here, caller reads
        /// them through the [`H2ConnectStream`] `AsyncRead` impl.
        body_tx: mpsc::Sender<io::Result<Bytes>>,
    },
}

#[cfg(test)]
mod flow_control_tests {
    //! `WINDOW_UPDATE`, SETTINGS, and the handshake path all reuse the
    //! same flow-window math via `checked_window_add`, giving all three
    //! call sites a single unit-test gate. If this helper ever returns
    //! `Ok` for a post-cap value, three RFC 9113 §6.9 invariants collapse
    //! simultaneously.

    use super::{checked_window_add, MAX_FLOW_WINDOW};

    #[test]
    fn exact_cap_is_ok() {
        assert_eq!(checked_window_add(0, MAX_FLOW_WINDOW), Ok(MAX_FLOW_WINDOW));
        assert_eq!(
            checked_window_add(65_535, MAX_FLOW_WINDOW - 65_535),
            Ok(MAX_FLOW_WINDOW)
        );
    }

    #[test]
    fn cap_plus_one_errors_with_post_add_value() {
        assert_eq!(
            checked_window_add(MAX_FLOW_WINDOW, 1),
            Err(MAX_FLOW_WINDOW + 1)
        );
    }

    #[test]
    fn two_max_increments_reject() {
        // Classic CVE-shape attack: WINDOW_UPDATE(0, 0x7FFFFFFF) twice
        // climbs the accumulator past the 2^31-1 ceiling.
        let step1 = checked_window_add(0, MAX_FLOW_WINDOW).unwrap();
        assert!(checked_window_add(step1, MAX_FLOW_WINDOW).is_err());
    }

    #[test]
    fn negative_delta_from_settings_is_permitted() {
        // RFC 9113 §6.9.2: a SETTINGS frame can drive the stream
        // window *negative*. The helper must NOT confuse that with
        // overflow — only the positive-cap is enforced.
        assert_eq!(checked_window_add(10_000, -20_000), Ok(-10_000));
        assert_eq!(
            checked_window_add(MAX_FLOW_WINDOW, -(MAX_FLOW_WINDOW + 1)),
            Ok(-1)
        );
    }

    #[test]
    fn saturating_add_prevents_signed_overflow() {
        // A genuinely pathological peer sending multiple max-sized
        // increments must not crash us via i64 overflow before the
        // cap check — `saturating_add` pins to i64::MAX which is
        // still > MAX_FLOW_WINDOW and trips the error branch.
        let res = checked_window_add(i64::MAX - 10, 100);
        assert!(res.is_err());
    }
}
