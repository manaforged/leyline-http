//! Per-stream state machine for an HTTP/2 client (RFC 9113 §5.1).

use crate::h2::error::ErrorCode;

/// Why a stream ended up in the `Closed` state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosedReason {
    /// Stream closed cleanly because both sides sent END_STREAM.
    EndStream,
    /// The client sent RST_STREAM to cancel the stream.
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
    /// The client sent END_STREAM; the server is still sending.
    HalfClosedLocal,
    /// The server sent END_STREAM; the client is still sending.
    HalfClosedRemote,
    /// Stream is finished; no further frames are valid.
    Closed {
        /// The reason this stream terminated.
        reason: ClosedReason,
    },
}

/// Events that drive stream state transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEvent {
    /// The client sent a HEADERS frame (request headers, not trailers).
    SendHeaders {
        /// Whether the HEADERS frame carried END_STREAM.
        end_stream: bool,
    },
    /// The client sent a DATA frame.
    SendData {
        /// Whether the DATA frame carried END_STREAM.
        end_stream: bool,
    },
    /// The client sent request trailers (a HEADERS frame after DATA, with END_STREAM).
    SendTrailers,
    /// The client sent a RST_STREAM frame with the given error code.
    SendRstStream(ErrorCode),
    /// Received a HEADERS frame (response headers).
    RecvHeaders {
        /// Whether the HEADERS frame carried END_STREAM.
        end_stream: bool,
    },
    /// Received a DATA frame.
    RecvData {
        /// Whether the DATA frame carried END_STREAM.
        end_stream: bool,
    },
    /// Received response trailers (a HEADERS frame after DATA, implicitly ending the stream).
    RecvTrailers,
    /// Received a RST_STREAM frame with the given error code.
    RecvRstStream(ErrorCode),
}

/// An illegal state transition was attempted.
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
    pub fn transition(&mut self, event: StreamEvent) -> Result<(), StreamStateError> {
        let next = next_state(*self, event)?;
        *self = next;
        Ok(())
    }

    /// Return true iff the stream is in `Closed` state.
    pub fn is_closed(&self) -> bool {
        matches!(self, StreamState::Closed { .. })
    }

    /// Return true iff we (the client) are still allowed to send DATA on this stream.
    pub fn can_send_data(&self) -> bool {
        matches!(self, StreamState::Open | StreamState::HalfClosedRemote)
    }

    /// Return true iff we (the client) are still allowed to receive DATA from the peer.
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
            E::SendHeaders { end_stream: false } => Ok(S::Open),
            E::SendHeaders { end_stream: true } => Ok(S::HalfClosedLocal),
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
            _ => Err(invalid(from, event)),
        },
        S::HalfClosedLocal => match event {
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
            E::SendData { end_stream: false } => Ok(S::HalfClosedRemote),
            E::SendData { end_stream: true } => Ok(S::Closed {
                reason: ClosedReason::EndStream,
            }),
            E::SendTrailers => Ok(S::Closed {
                reason: ClosedReason::EndStream,
            }),
            _ => Err(invalid(from, event)),
        },
        S::Closed { .. } => Err(invalid(from, event)),
    }
}
