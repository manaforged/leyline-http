use std::io;
use std::time::Duration;

use tokio::time::Instant;

use super::waiting;

#[test]
fn an_interrupted_call_is_retried_until_it_finishes() {
    let mut calls = 0;
    let mut applied = None;
    let out = waiting(
        &mut applied,
        Instant::now(),
        None,
        |_| Ok(()),
        || {
            calls += 1;
            match calls {
                1 | 2 => Err(io::Error::from(io::ErrorKind::Interrupted)),
                _ => Ok(calls),
            }
        },
    );

    assert_eq!(out.expect("third call succeeds"), 3);
}

#[test]
fn repeated_interrupts_still_end_at_the_deadline() {
    let started = Instant::now();
    let mut applied = None;
    let shrinking = std::cell::Cell::new(Duration::MAX);
    let sets = std::cell::Cell::new(0);
    let out = waiting(
        &mut applied,
        started,
        Some(started + Duration::from_millis(100)),
        |limit| {
            let limit = limit.expect("a deadline sets a socket timeout");
            assert!(limit < shrinking.get());
            shrinking.set(limit);
            sets.set(sets.get() + 1);
            Ok(())
        },
        || {
            pause(Duration::from_millis(30));
            Err::<(), _>(io::Error::from(io::ErrorKind::Interrupted))
        },
    );

    assert_eq!(
        out.expect_err("the deadline passes").kind(),
        io::ErrorKind::TimedOut
    );
    assert!(started.elapsed() < Duration::from_millis(200));
    assert!(sets.get() > 1);
}

#[test]
fn any_other_error_is_returned_at_once() {
    let mut calls = 0;
    let mut applied = None;
    let out = waiting(
        &mut applied,
        Instant::now(),
        None,
        |_| Ok(()),
        || {
            calls += 1;
            Err::<(), _>(io::Error::from(io::ErrorKind::ConnectionReset))
        },
    );

    assert_eq!(
        out.expect_err("the error passes through").kind(),
        io::ErrorKind::ConnectionReset
    );
    assert_eq!(calls, 1);
}

fn pause(wait: std::time::Duration) {
    let (_keep, rx) = std::sync::mpsc::channel::<()>();
    let _elapsed = rx.recv_timeout(wait);
}
