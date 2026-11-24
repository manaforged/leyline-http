use http::StatusCode;

use super::{MAX_INFORMATIONAL, ResponseState};

fn any_state() -> ResponseState {
    match kani::any::<u8>() % 3 {
        0 => ResponseState::Initial(kani::any_where(|&n| n <= MAX_INFORMATIONAL + 2)),
        1 => ResponseState::Final,
        _ => ResponseState::Trailers,
    }
}

#[kani::proof]
fn response_state_rejects_101_and_caps_informational_responses() {
    let mut state = any_state();
    let before = state;
    let code: u16 = kani::any_where(|&c| (100..=999).contains(&c));
    let status = StatusCode::from_u16(code).expect("valid status code");
    let res = state.head(status);
    match before {
        ResponseState::Initial(_) if code == 101 => {
            assert!(res.is_err());
            assert_eq!(state, before);
        }
        ResponseState::Initial(n) if (100..200).contains(&code) => {
            if n < MAX_INFORMATIONAL {
                assert!(res.is_ok());
                assert_eq!(state, ResponseState::Initial(n + 1));
            } else {
                assert!(res.is_err());
                assert_eq!(state, before);
            }
        }
        ResponseState::Initial(_) => {
            assert!(res.is_ok());
            assert_eq!(state, ResponseState::Final);
        }
        ResponseState::Final | ResponseState::Trailers => {
            assert!(res.is_err());
            assert_eq!(state, before);
        }
    }
}

#[kani::proof]
fn response_state_data_trailers_and_finish_enforce_lifecycle() {
    let mut state = any_state();
    let before = state;
    assert_eq!(state.data().is_ok(), matches!(before, ResponseState::Final));
    assert_eq!(
        state.finish().is_ok(),
        matches!(before, ResponseState::Final | ResponseState::Trailers)
    );
    let res = state.trailers();
    match before {
        ResponseState::Final => {
            assert!(res.is_ok());
            assert_eq!(state, ResponseState::Trailers);
        }
        ResponseState::Initial(_) | ResponseState::Trailers => {
            assert!(res.is_err());
            assert_eq!(state, before);
        }
    }
}
