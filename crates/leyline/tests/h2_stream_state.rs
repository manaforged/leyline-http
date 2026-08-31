//! Pure state-machine tests for `StreamState`.

#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
#![expect(
    clippy::panic,
    reason = "test harness helper: explicit panic on unexpected error shape is the assertion"
)]
use leyline::h2::error::ErrorCode;
use leyline::h2::stream_state::{ClosedReason, StreamEvent, StreamState, StreamStateError};

fn idle() -> StreamState {
    StreamState::Idle
}

fn open() -> StreamState {
    let mut s = idle();
    s.transition(StreamEvent::SendHeaders { end_stream: false })
        .unwrap();
    s
}

fn half_closed_local() -> StreamState {
    let mut s = idle();
    s.transition(StreamEvent::SendHeaders { end_stream: true })
        .unwrap();
    s
}

fn half_closed_remote() -> StreamState {
    let mut s = open();
    s.transition(StreamEvent::RecvHeaders { end_stream: true })
        .unwrap();
    s
}

#[test]
fn idle_to_open_via_send_headers() {
    let mut s = idle();
    s.transition(StreamEvent::SendHeaders { end_stream: false })
        .unwrap();
    assert_eq!(s, StreamState::Open);
}

#[test]
fn idle_to_half_closed_local_via_send_headers_end_stream() {
    let mut s = idle();
    s.transition(StreamEvent::SendHeaders { end_stream: true })
        .unwrap();
    assert_eq!(s, StreamState::HalfClosedLocal);
}

#[test]
fn open_send_data_no_end_stream_stays_open() {
    let mut s = open();
    s.transition(StreamEvent::SendData { end_stream: false })
        .unwrap();
    assert_eq!(s, StreamState::Open);
}

#[test]
fn open_send_data_end_stream_to_half_closed_local() {
    let mut s = open();
    s.transition(StreamEvent::SendData { end_stream: true })
        .unwrap();
    assert_eq!(s, StreamState::HalfClosedLocal);
}

#[test]
fn open_send_trailers_to_half_closed_local() {
    let mut s = open();
    s.transition(StreamEvent::SendTrailers).unwrap();
    assert_eq!(s, StreamState::HalfClosedLocal);
}

#[test]
fn open_recv_headers_no_end_stream_stays_open() {
    let mut s = open();
    s.transition(StreamEvent::RecvHeaders { end_stream: false })
        .unwrap();
    assert_eq!(s, StreamState::Open);
}

#[test]
fn open_recv_headers_end_stream_to_half_closed_remote() {
    let mut s = open();
    s.transition(StreamEvent::RecvHeaders { end_stream: true })
        .unwrap();
    assert_eq!(s, StreamState::HalfClosedRemote);
}

#[test]
fn open_recv_data_no_end_stream_stays_open() {
    let mut s = open();
    s.transition(StreamEvent::RecvData { end_stream: false })
        .unwrap();
    assert_eq!(s, StreamState::Open);
}

#[test]
fn open_recv_data_end_stream_to_half_closed_remote() {
    let mut s = open();
    s.transition(StreamEvent::RecvData { end_stream: true })
        .unwrap();
    assert_eq!(s, StreamState::HalfClosedRemote);
}

#[test]
fn open_recv_trailers_to_half_closed_remote() {
    let mut s = open();
    s.transition(StreamEvent::RecvTrailers).unwrap();
    assert_eq!(s, StreamState::HalfClosedRemote);
}

#[test]
fn half_closed_local_recv_headers_no_end_stream_stays() {
    let mut s = half_closed_local();
    s.transition(StreamEvent::RecvHeaders { end_stream: false })
        .unwrap();
    assert_eq!(s, StreamState::HalfClosedLocal);
}

#[test]
fn half_closed_local_recv_headers_end_stream_to_closed() {
    let mut s = half_closed_local();
    s.transition(StreamEvent::RecvHeaders { end_stream: true })
        .unwrap();
    assert_eq!(
        s,
        StreamState::Closed {
            reason: ClosedReason::EndStream
        }
    );
}

#[test]
fn half_closed_local_recv_data_no_end_stream_stays() {
    let mut s = half_closed_local();
    s.transition(StreamEvent::RecvData { end_stream: false })
        .unwrap();
    assert_eq!(s, StreamState::HalfClosedLocal);
}

#[test]
fn half_closed_local_recv_data_end_stream_to_closed() {
    let mut s = half_closed_local();
    s.transition(StreamEvent::RecvData { end_stream: true })
        .unwrap();
    assert_eq!(
        s,
        StreamState::Closed {
            reason: ClosedReason::EndStream
        }
    );
}

#[test]
fn half_closed_local_recv_trailers_to_closed() {
    let mut s = half_closed_local();
    s.transition(StreamEvent::RecvTrailers).unwrap();
    assert_eq!(
        s,
        StreamState::Closed {
            reason: ClosedReason::EndStream
        }
    );
}

#[test]
fn half_closed_remote_send_data_no_end_stream_stays() {
    let mut s = half_closed_remote();
    s.transition(StreamEvent::SendData { end_stream: false })
        .unwrap();
    assert_eq!(s, StreamState::HalfClosedRemote);
}

#[test]
fn half_closed_remote_send_data_end_stream_to_closed() {
    let mut s = half_closed_remote();
    s.transition(StreamEvent::SendData { end_stream: true })
        .unwrap();
    assert_eq!(
        s,
        StreamState::Closed {
            reason: ClosedReason::EndStream
        }
    );
}

#[test]
fn half_closed_remote_send_trailers_to_closed() {
    let mut s = half_closed_remote();
    s.transition(StreamEvent::SendTrailers).unwrap();
    assert_eq!(
        s,
        StreamState::Closed {
            reason: ClosedReason::EndStream
        }
    );
}

#[test]
fn rst_local_from_open_closes_with_rst_local() {
    let mut s = open();
    s.transition(StreamEvent::SendRstStream(ErrorCode::Cancel))
        .unwrap();
    assert_eq!(
        s,
        StreamState::Closed {
            reason: ClosedReason::RstLocal(ErrorCode::Cancel)
        }
    );
}

#[test]
fn rst_remote_from_open_closes_with_rst_remote() {
    let mut s = open();
    s.transition(StreamEvent::RecvRstStream(ErrorCode::RefusedStream))
        .unwrap();
    assert_eq!(
        s,
        StreamState::Closed {
            reason: ClosedReason::RstRemote(ErrorCode::RefusedStream)
        }
    );
}

#[test]
fn rst_local_from_half_closed_local_closes() {
    let mut s = half_closed_local();
    s.transition(StreamEvent::SendRstStream(ErrorCode::Cancel))
        .unwrap();
    assert!(s.is_closed());
}

#[test]
fn rst_remote_from_half_closed_remote_closes() {
    let mut s = half_closed_remote();
    s.transition(StreamEvent::RecvRstStream(ErrorCode::InternalError))
        .unwrap();
    assert!(s.is_closed());
}

fn assert_invalid(res: Result<(), StreamStateError>) {
    match res {
        Err(StreamStateError::InvalidTransition { .. }) => {}
        other => panic!("expected InvalidTransition, got {:?}", other),
    }
}

#[test]
fn idle_rejects_send_data() {
    let mut s = idle();
    assert_invalid(s.transition(StreamEvent::SendData { end_stream: false }));
    assert_eq!(s, StreamState::Idle);
}

#[test]
fn idle_rejects_recv_headers() {
    let mut s = idle();
    assert_invalid(s.transition(StreamEvent::RecvHeaders { end_stream: false }));
}

#[test]
fn idle_rejects_recv_data() {
    let mut s = idle();
    assert_invalid(s.transition(StreamEvent::RecvData { end_stream: false }));
}

#[test]
fn idle_rejects_send_rst_stream() {
    let mut s = idle();
    assert_invalid(s.transition(StreamEvent::SendRstStream(ErrorCode::Cancel)));
}

#[test]
fn idle_rejects_recv_rst_stream() {
    let mut s = idle();
    assert_invalid(s.transition(StreamEvent::RecvRstStream(ErrorCode::Cancel)));
}

#[test]
fn idle_rejects_send_trailers() {
    let mut s = idle();
    assert_invalid(s.transition(StreamEvent::SendTrailers));
}

#[test]
fn open_rejects_send_headers_again() {
    let mut s = open();
    assert_invalid(s.transition(StreamEvent::SendHeaders { end_stream: false }));
}

#[test]
fn closed_rejects_send_data() {
    let mut s = half_closed_local();
    s.transition(StreamEvent::RecvData { end_stream: true })
        .unwrap();
    assert!(s.is_closed());
    assert_invalid(s.transition(StreamEvent::SendData { end_stream: false }));
}

#[test]
fn closed_rejects_recv_data() {
    let mut s = half_closed_local();
    s.transition(StreamEvent::RecvData { end_stream: true })
        .unwrap();
    assert_invalid(s.transition(StreamEvent::RecvData { end_stream: false }));
}

#[test]
fn recv_trailers_before_recv_headers_is_invalid_from_idle() {
    let mut s = idle();
    assert_invalid(s.transition(StreamEvent::RecvTrailers));
}

#[test]
fn double_end_stream_recv_is_invalid() {
    let mut s = half_closed_local();
    s.transition(StreamEvent::RecvData { end_stream: true })
        .unwrap();
    assert_invalid(s.transition(StreamEvent::RecvData { end_stream: true }));
}

#[test]
fn double_end_stream_send_is_invalid() {
    let mut s = half_closed_local();
    assert_invalid(s.transition(StreamEvent::SendData { end_stream: true }));
}

#[test]
fn half_closed_local_rejects_send_data() {
    let mut s = half_closed_local();
    assert_invalid(s.transition(StreamEvent::SendData { end_stream: false }));
}

#[test]
fn half_closed_remote_rejects_recv_data() {
    let mut s = half_closed_remote();
    assert_invalid(s.transition(StreamEvent::RecvData { end_stream: false }));
}

#[test]
fn half_closed_remote_rejects_recv_trailers() {
    let mut s = half_closed_remote();
    assert_invalid(s.transition(StreamEvent::RecvTrailers));
}

#[test]
fn invalid_transition_name_populated() {
    let mut s = idle();
    let err = s
        .transition(StreamEvent::SendData { end_stream: false })
        .unwrap_err();
    match err {
        StreamStateError::InvalidTransition { from, event } => {
            assert_eq!(from, "Idle");
            assert_eq!(event, "SendData");
        }
    }
}

#[test]
fn is_closed_flags() {
    assert!(!idle().is_closed());
    assert!(!open().is_closed());
    assert!(!half_closed_local().is_closed());
    assert!(!half_closed_remote().is_closed());
    assert!(
        StreamState::Closed {
            reason: ClosedReason::EndStream
        }
        .is_closed()
    );
    assert!(
        StreamState::Closed {
            reason: ClosedReason::Error
        }
        .is_closed()
    );
}

#[test]
fn can_send_data_only_when_local_side_open() {
    assert!(!idle().can_send_data());
    assert!(open().can_send_data());
    assert!(!half_closed_local().can_send_data());
    assert!(half_closed_remote().can_send_data());
    assert!(
        !StreamState::Closed {
            reason: ClosedReason::EndStream
        }
        .can_send_data()
    );
}

#[test]
fn can_recv_data_only_when_remote_side_open() {
    assert!(!idle().can_recv_data());
    assert!(open().can_recv_data());
    assert!(half_closed_local().can_recv_data());
    assert!(!half_closed_remote().can_recv_data());
    assert!(
        !StreamState::Closed {
            reason: ClosedReason::EndStream
        }
        .can_recv_data()
    );
}

#[test]
fn closed_reasons_are_preserved() {
    let mut s = open();
    s.transition(StreamEvent::SendRstStream(ErrorCode::Cancel))
        .unwrap();
    match s {
        StreamState::Closed {
            reason: ClosedReason::RstLocal(ErrorCode::Cancel),
        } => {}
        other => panic!("wrong closed reason: {:?}", other),
    }

    let mut s = open();
    s.transition(StreamEvent::RecvRstStream(ErrorCode::RefusedStream))
        .unwrap();
    match s {
        StreamState::Closed {
            reason: ClosedReason::RstRemote(ErrorCode::RefusedStream),
        } => {}
        other => panic!("wrong closed reason: {:?}", other),
    }
}
