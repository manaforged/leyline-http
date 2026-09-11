use crate::h2::error::ErrorCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosedReason {
    EndStream,
    RstLocal(ErrorCode),
    RstRemote(ErrorCode),
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamState {
    Idle,
    Open,
    HalfClosedLocal,
    HalfClosedRemote,
    Closed { reason: ClosedReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEvent {
    SendHeaders { end_stream: bool },
    SendData { end_stream: bool },
    SendTrailers,
    SendRstStream(ErrorCode),
    RecvHeaders { end_stream: bool },
    RecvData { end_stream: bool },
    RecvTrailers,
    RecvRstStream(ErrorCode),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StreamStateError {
    #[error("invalid transition: cannot apply {event} in state {from}")]
    InvalidTransition {
        from: &'static str,
        event: &'static str,
    },
}

impl StreamState {
    pub fn transition(&mut self, event: StreamEvent) -> Result<(), StreamStateError> {
        let next = next_state(*self, event)?;
        *self = next;
        Ok(())
    }

    pub fn is_closed(&self) -> bool {
        matches!(self, StreamState::Closed { .. })
    }

    pub fn can_send_data(&self) -> bool {
        matches!(self, StreamState::Open | StreamState::HalfClosedRemote)
    }

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
