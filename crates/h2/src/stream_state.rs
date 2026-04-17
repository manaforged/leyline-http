//! Per-stream state machine for an HTTP/2 client (RFC 9113 §5.1).
//!
//! This module models stream lifecycle from the client's perspective only.
//! We never reach `ReservedLocal` / `ReservedRemote`: the client disables
//! `SETTINGS_ENABLE_PUSH` (or RST_STREAMs any PUSH_PROMISE it sees), so
//! server-initiated reserved states cannot arise during a client session.

use crate::error::ErrorCode;

/// Why a stream ended up in the `Closed` state.
///
/// Kept around for the lifetime of whatever stream-info record the
/// connection still holds, so callers can reason about *why* a stream is
/// gone (clean end-of-stream vs. cancellation vs. error).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosedReason {
    /// Stream closed cleanly because both sides sent END_STREAM.
    EndStream,
    /// We sent RST_STREAM to cancel the stream.
    RstLocal(ErrorCode),
    /// Peer sent RST_STREAM to cancel the stream.
    RstRemote(ErrorCode),
    /// Stream ended because of a protocol/state-machine violation.
    Error,
}

/// RFC 9113 §5.1 stream state, from the client perspective.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamState {
    /// Stream id has not been used yet — no frames sent or received.
    Idle,
    /// Both endpoints may send frames freely.
    Open,
    /// We sent END_STREAM; server is still sending.
    HalfClosedLocal,
    /// Server sent END_STREAM; we are still sending. Rare for a client.
    HalfClosedRemote,
    /// Stream is finished; no further frames are valid.
    Closed {
        /// The reason this stream terminated.
        reason: ClosedReason,
    },
}

/// Events that drive stream state transitions.
///
/// These are the *logical* events the connection loop fires when it either
/// writes a frame to the peer or hands an inbound frame off to the state
/// machine. Flow control accounting is orthogonal and lives on
/// `ClientConnection` directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEvent {
    /// We sent a HEADERS frame (request headers, not trailers).
    SendHeaders {
        /// Whether the HEADERS frame carried END_STREAM.
        end_stream: bool,
    },
    /// We sent a DATA frame.
    SendData {
        /// Whether the DATA frame carried END_STREAM.
        end_stream: bool,
    },
    /// We sent request trailers (a HEADERS frame after DATA, with END_STREAM).
    SendTrailers,
    /// We sent a RST_STREAM frame with the given error code.
    SendRstStream(ErrorCode),
    /// We received a HEADERS frame (response headers).
    RecvHeaders {
        /// Whether the HEADERS frame carried END_STREAM.
        end_stream: bool,
    },
    /// We received a DATA frame.
    RecvData {
        /// Whether the DATA frame carried END_STREAM.
        end_stream: bool,
    },
    /// We received response trailers (a HEADERS frame after DATA, implicitly ending the stream).
    RecvTrailers,
    /// We received a RST_STREAM frame with the given error code.
    RecvRstStream(ErrorCode),
}

/// An illegal state transition was attempted.
///
/// Call sites map this into `H2Error::Stream { code: ProtocolError, .. }`
/// before surfacing it to the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StreamStateError {
    /// The state machine refused the event.
    #[error("invalid transition: cannot apply {event} in state {from}")]
    InvalidTransition {
        /// Human-readable name of the source state.
        from: &'static str,
        /// Human-readable name of the event.
        event: &'static str,
    },
}

impl StreamState {
    /// Apply an event to this stream, mutating in place on success.
    ///
    /// On success, `self` reflects the new state. On failure, `self` is
    /// left untouched and an `InvalidTransition` error is returned —
    /// callers are expected to promote this to a connection/stream
    /// protocol error.
    pub fn transition(&mut self, event: StreamEvent) -> Result<(), StreamStateError> {
        let next = next_state(*self, event)?;
        *self = next;
        Ok(())
    }

    /// Return true iff the stream is in `Closed` state.
    pub fn is_closed(&self) -> bool {
        matches!(self, StreamState::Closed { .. })
    }

    /// Return true iff we (the client) are still allowed to send DATA
    /// on this stream.
    pub fn can_send_data(&self) -> bool {
        matches!(self, StreamState::Open | StreamState::HalfClosedRemote)
    }

    /// Return true iff we (the client) are still allowed to receive
    /// DATA from the peer.
    pub fn can_recv_data(&self) -> bool {
        matches!(self, StreamState::Open | StreamState::HalfClosedLocal)
    }
}

fn state_name(state: StreamState) -> &'static str {
    match state {
        StreamState::Idle => "Idle",
        StreamState::Open => "Open",
        StreamState::HalfClosedLocal => "HalfClosedLocal",
        StreamState::HalfClosedRemote => "HalfClosedRemote",
        StreamState::Closed { .. } => "Closed",
    }
}

fn event_name(event: StreamEvent) -> &'static str {
    match event {
        StreamEvent::SendHeaders { .. } => "SendHeaders",
        StreamEvent::SendData { .. } => "SendData",
        StreamEvent::SendTrailers => "SendTrailers",
        StreamEvent::SendRstStream(_) => "SendRstStream",
        StreamEvent::RecvHeaders { .. } => "RecvHeaders",
        StreamEvent::RecvData { .. } => "RecvData",
        StreamEvent::RecvTrailers => "RecvTrailers",
        StreamEvent::RecvRstStream(_) => "RecvRstStream",
    }
}

fn invalid(from: StreamState, event: StreamEvent) -> StreamStateError {
    StreamStateError::InvalidTransition {
        from: state_name(from),
        event: event_name(event),
    }
}

fn next_state(from: StreamState, event: StreamEvent) -> Result<StreamState, StreamStateError> {
    use StreamEvent as E;
    use StreamState as S;

    // RST_STREAM either way terminates the stream from any non-idle state.
    // Idle + RST is a protocol violation per RFC 9113 §5.1.
    match (from, event) {
        (S::Idle, E::SendRstStream(_)) | (S::Idle, E::RecvRstStream(_)) => {
            return Err(invalid(from, event));
        }
        (_, E::SendRstStream(code)) => {
            return Ok(S::Closed {
                reason: ClosedReason::RstLocal(code),
            });
        }
        (_, E::RecvRstStream(code)) => {
            return Ok(S::Closed {
                reason: ClosedReason::RstRemote(code),
            });
        }
        _ => {}
    }

    match from {
        S::Idle => match event {
            // Opening HEADERS — with or without END_STREAM.
            E::SendHeaders { end_stream: false } => Ok(S::Open),
            E::SendHeaders { end_stream: true } => Ok(S::HalfClosedLocal),
            // Everything else on an idle stream is a protocol error.
            _ => Err(invalid(from, event)),
        },
        S::Open => match event {
            E::SendData { end_stream: false } => Ok(S::Open),
            E::SendData { end_stream: true } => Ok(S::HalfClosedLocal),
            E::SendTrailers => Ok(S::HalfClosedLocal),
            E::RecvHeaders { end_stream: false } => Ok(S::Open),
            E::RecvHeaders { end_stream: true } => Ok(S::HalfClosedRemote),
            E::RecvData { end_stream: false } => Ok(S::Open),
            E::RecvData { end_stream: true } => Ok(S::HalfClosedRemote),
            E::RecvTrailers => Ok(S::HalfClosedRemote),
            // SendHeaders again in Open would be trailers; require SendTrailers.
            _ => Err(invalid(from, event)),
        },
        S::HalfClosedLocal => match event {
            // We already sent END_STREAM — no more outbound DATA/HEADERS.
            E::RecvHeaders { end_stream: false } => Ok(S::HalfClosedLocal),
            E::RecvHeaders { end_stream: true } => Ok(S::Closed {
                reason: ClosedReason::EndStream,
            }),
            E::RecvData { end_stream: false } => Ok(S::HalfClosedLocal),
            E::RecvData { end_stream: true } => Ok(S::Closed {
                reason: ClosedReason::EndStream,
            }),
            E::RecvTrailers => Ok(S::Closed {
                reason: ClosedReason::EndStream,
            }),
            _ => Err(invalid(from, event)),
        },
        S::HalfClosedRemote => match event {
            // Server already end-streamed; we can still finish sending.
            E::SendData { end_stream: false } => Ok(S::HalfClosedRemote),
            E::SendData { end_stream: true } => Ok(S::Closed {
                reason: ClosedReason::EndStream,
            }),
            E::SendTrailers => Ok(S::Closed {
                reason: ClosedReason::EndStream,
            }),
            _ => Err(invalid(from, event)),
        },
        S::Closed { .. } => {
            // RST_STREAM was handled above; everything else is a violation.
            // RFC 9113 §5.1 permits *tolerating* some frames on recently
            // closed streams — that tolerance is implemented at the call
            // site (see `ClientConnection::transition_stream`), not here.
            Err(invalid(from, event))
        }
    }
}
