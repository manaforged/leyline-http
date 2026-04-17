//! RST_STREAM flood detector tests (CVE-2023-44487 defense-in-depth).
//!
//! Pure state tests — no tokio, no network — we drive the sliding window
//! directly with synthetic timestamps via `RstFloodDetector::record`.

use std::time::{Duration, Instant};

use leyline_h2::connection::RstFloodDetector;
use leyline_h2::error::ErrorCode;
use leyline_h2::H2Error;

fn assert_calm(err: &H2Error) {
    match err {
        H2Error::Connection { code, .. } => {
            assert_eq!(*code, ErrorCode::EnhanceYourCalm, "wrong error code");
        }
        other => panic!("expected Connection error, got {other:?}"),
    }
}

#[test]
fn threshold_plus_one_trips_flood_guard() {
    let mut d = RstFloodDetector::new(100, Duration::from_secs(10));
    let base = Instant::now();
    // 100 events at t=0ms…99ms all inside the window and under/at threshold.
    for i in 0..100 {
        d.record(base + Duration::from_millis(i)).unwrap();
    }
    // The 101st RST inside the window tips us over.
    let err = d
        .record(base + Duration::from_millis(100))
        .expect_err("101st RST must trip the guard");
    assert_calm(&err);
}

#[test]
fn events_outside_window_are_evicted() {
    let mut d = RstFloodDetector::new(5, Duration::from_secs(1));
    let base = Instant::now();
    // Five old events — each 2s apart — all land outside the 1s window
    // by the time the newer batch arrives.
    for i in 0..5 {
        d.record(base + Duration::from_millis(i)).unwrap();
    }
    // Jump 5s ahead — the old events age out. We should be able to record
    // another 5 fresh events without tripping the guard.
    let future = base + Duration::from_secs(5);
    for i in 0..5 {
        d.record(future + Duration::from_millis(i)).unwrap();
    }
    // The 6th fresh event inside the 1s window does trip the guard.
    let err = d
        .record(future + Duration::from_millis(10))
        .expect_err("6th burst event must trip guard");
    assert_calm(&err);
}

#[test]
fn under_threshold_never_trips() {
    let mut d = RstFloodDetector::new(10, Duration::from_secs(10));
    let base = Instant::now();
    for i in 0..10 {
        d.record(base + Duration::from_millis(i * 50))
            .expect("under threshold must not trip");
    }
}
