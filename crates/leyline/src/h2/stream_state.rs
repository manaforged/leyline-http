use crate::h2::error::ErrorCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosedReason {
    EndStream,
    RstRemote(ErrorCode),
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
        let next = self.next(event)?;
        *self = next;
        Ok(())
    }

    pub fn is_closed(&self) -> bool {
        matches!(self, StreamState::Closed { .. })
    }

    fn name(self) -> &'static str {
        match self {
            StreamState::Idle => "Idle",
            StreamState::Open => "Open",
            StreamState::HalfClosedLocal => "HalfClosedLocal",
            StreamState::HalfClosedRemote => "HalfClosedRemote",
            StreamState::Closed { .. } => "Closed",
        }
    }
}

impl StreamEvent {
    fn name(self) -> &'static str {
        match self {
            StreamEvent::SendHeaders { .. } => "SendHeaders",
            StreamEvent::SendData { .. } => "SendData",
            StreamEvent::RecvHeaders { .. } => "RecvHeaders",
            StreamEvent::RecvData { .. } => "RecvData",
            StreamEvent::RecvTrailers => "RecvTrailers",
            StreamEvent::RecvRstStream(_) => "RecvRstStream",
        }
    }
}

impl StreamState {
    fn invalid(self, event: StreamEvent) -> StreamStateError {
        StreamStateError::InvalidTransition {
            from: self.name(),
            event: event.name(),
        }
    }

    fn next(self, event: StreamEvent) -> Result<StreamState, StreamStateError> {
        use StreamEvent as E;
        use StreamState as S;

        match (self, event) {
            (S::Idle, E::RecvRstStream(_)) => {
                return Err(self.invalid(event));
            }
            (_, E::RecvRstStream(code)) => {
                return Ok(S::Closed {
                    reason: ClosedReason::RstRemote(code),
                });
            }
            _ => {}
        }

        match self {
            S::Idle => match event {
                E::SendHeaders { end_stream: false } => Ok(S::Open),
                E::SendHeaders { end_stream: true } => Ok(S::HalfClosedLocal),
                _ => Err(self.invalid(event)),
            },
            S::Open => match event {
                E::SendData { end_stream: false } => Ok(S::Open),
                E::SendData { end_stream: true } => Ok(S::HalfClosedLocal),
                E::RecvHeaders { end_stream: false } => Ok(S::Open),
                E::RecvHeaders { end_stream: true } => Ok(S::HalfClosedRemote),
                E::RecvData { end_stream: false } => Ok(S::Open),
                E::RecvData { end_stream: true } => Ok(S::HalfClosedRemote),
                E::RecvTrailers => Ok(S::HalfClosedRemote),
                _ => Err(self.invalid(event)),
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
                _ => Err(self.invalid(event)),
            },
            S::HalfClosedRemote => match event {
                E::SendData { end_stream: false } => Ok(S::HalfClosedRemote),
                E::SendData { end_stream: true } => Ok(S::Closed {
                    reason: ClosedReason::EndStream,
                }),
                _ => Err(self.invalid(event)),
            },
            S::Closed { .. } => Err(self.invalid(event)),
        }
    }
}
