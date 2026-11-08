use super::{ClosedReason, StreamEvent, StreamState};
use crate::h2::error::ErrorCode;

fn any_code() -> ErrorCode {
    ErrorCode::from_u32(kani::any())
}

fn any_state() -> StreamState {
    match kani::any::<u8>() % 5 {
        0 => StreamState::Idle,
        1 => StreamState::Open,
        2 => StreamState::HalfClosedLocal,
        3 => StreamState::HalfClosedRemote,
        _ => StreamState::Closed {
            reason: match kani::any::<u8>() % 3 {
                0 => ClosedReason::EndStream,
                1 => ClosedReason::RstRemote(any_code()),
                _ => ClosedReason::RstLocal(any_code()),
            },
        },
    }
}

fn any_event() -> StreamEvent {
    let end_stream = kani::any();
    match kani::any::<u8>() % 7 {
        0 => StreamEvent::SendHeaders { end_stream },
        1 => StreamEvent::SendData { end_stream },
        2 => StreamEvent::RecvHeaders { end_stream },
        3 => StreamEvent::RecvData { end_stream },
        4 => StreamEvent::RecvTrailers,
        5 => StreamEvent::RecvRstStream(any_code()),
        _ => StreamEvent::SendRstStream(any_code()),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Halves {
    started: bool,
    local: bool,
    remote: bool,
}

fn halves(state: StreamState) -> Halves {
    let (started, local, remote) = match state {
        StreamState::Idle => (false, true, true),
        StreamState::Open => (true, true, true),
        StreamState::HalfClosedLocal => (true, false, true),
        StreamState::HalfClosedRemote => (true, true, false),
        StreamState::Closed { .. } => (true, false, false),
    };
    Halves {
        started,
        local,
        remote,
    }
}

enum Expected {
    Reject,
    Reset(ClosedReason),
    Move(Halves),
}

/// RFC 9113 section 5.1 for a client that opens streams with HEADERS and never sends trailers.
fn rfc_9113_section_5_1(state: StreamState, event: StreamEvent) -> Expected {
    let now = halves(state);
    let closed = now.started && !now.local && !now.remote;
    match event {
        StreamEvent::RecvRstStream(_) | StreamEvent::SendRstStream(_) if !now.started => {
            Expected::Reject
        }
        StreamEvent::RecvRstStream(code) => Expected::Reset(ClosedReason::RstRemote(code)),
        StreamEvent::SendRstStream(code) => Expected::Reset(ClosedReason::RstLocal(code)),
        _ if closed => Expected::Reject,
        StreamEvent::SendHeaders { end_stream } if !now.started => Expected::Move(Halves {
            started: true,
            local: !end_stream,
            remote: true,
        }),
        _ if !now.started => Expected::Reject,
        StreamEvent::SendHeaders { .. } => Expected::Reject,
        StreamEvent::SendData { end_stream } if now.local => Expected::Move(Halves {
            local: !end_stream,
            ..now
        }),
        StreamEvent::SendData { .. } => Expected::Reject,
        StreamEvent::RecvHeaders { end_stream } | StreamEvent::RecvData { end_stream }
            if now.remote =>
        {
            Expected::Move(Halves {
                remote: !end_stream,
                ..now
            })
        }
        StreamEvent::RecvTrailers if now.remote => Expected::Move(Halves {
            remote: false,
            ..now
        }),
        _ => Expected::Reject,
    }
}

#[kani::proof]
fn every_stream_transition_follows_rfc_9113_section_5_1() {
    let state = any_state();
    let event = any_event();
    let mut next = state;
    let result = next.transition(event);
    match rfc_9113_section_5_1(state, event) {
        Expected::Reject => {
            assert!(result.is_err());
            assert_eq!(next, state);
        }
        Expected::Reset(reason) => {
            assert!(result.is_ok());
            assert_eq!(next, StreamState::Closed { reason });
        }
        Expected::Move(want) => {
            assert!(result.is_ok());
            assert_eq!(halves(next), want);
            if !want.local && !want.remote {
                assert_eq!(
                    next,
                    StreamState::Closed {
                        reason: ClosedReason::EndStream
                    }
                );
            }
        }
    }
}
