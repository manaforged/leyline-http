#![expect(
    clippy::panic,
    reason = "test harness helper: explicit panic on unexpected error shape is the assertion"
)]
use std::time::{Duration, Instant};

use leyline::h2::H2Error;
use leyline::h2::connection::RstFloodDetector;
use leyline::h2::error::ErrorCode;

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
    for i in 0..100 {
        d.record(base + Duration::from_millis(i)).unwrap();
    }
    let err = d
        .record(base + Duration::from_millis(100))
        .expect_err("101st RST must trip the guard");
    assert_calm(&err);
}

#[test]
fn events_outside_window_are_evicted() {
    let mut d = RstFloodDetector::new(5, Duration::from_secs(1));
    let base = Instant::now();
    for i in 0..5 {
        d.record(base + Duration::from_millis(i)).unwrap();
    }
    let future = base + Duration::from_secs(5);
    for i in 0..5 {
        d.record(future + Duration::from_millis(i)).unwrap();
    }
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

#[test]
fn settings_flood_label_is_distinct() {
    let mut d = RstFloodDetector::with_label(
        3,
        Duration::from_secs(10),
        "leyline::h2::settings_flood",
        "peer sent excessive SETTINGS updates",
    );
    let base = Instant::now();
    for i in 0..3 {
        d.record(base + Duration::from_millis(i)).unwrap();
    }
    let err = d
        .record(base + Duration::from_millis(3))
        .expect_err("4th event must trip");
    match err {
        H2Error::Connection { code, reason } => {
            assert_eq!(code, ErrorCode::EnhanceYourCalm);
            assert!(
                reason.contains("SETTINGS"),
                "reason string should reflect the detector's label: {reason}"
            );
        }
        other => panic!("expected Connection error, got {other:?}"),
    }
}
