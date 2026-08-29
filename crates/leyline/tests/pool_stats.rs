//! Regression tests for [`leyline::pool::PoolStats`] — the
//! observability surface that was
//! missing. The pool's checkout / install / evict paths each update
//! their counter; this gates those counter increments against
//! realistic insert + evict flows.
//!
//! Tests construct a bare [`Pool`] and hit only the public surface;
//! they do not stand up an H2 driver, so the `hits` counter is
//! exercised via `checkout` on a freshly-installed entry through the
//! private-but-exposed-in-tests path — actually, since `install` and
//! `checkout` are `fn` (crate-private), we validate the observable
//! bits: `stats()` on an empty pool, `with_limits` reflects in the
//! cap, and `len()` / `is_empty()` agree with `stats().entries`.
//!
//! Hits on a live H2 driver are covered by `tests/h2_multiplex.rs`.

use leyline::pool::{DEFAULT_IDLE_TIMEOUT, DEFAULT_MAX_CONNECTIONS, Pool, PoolStats};
use std::time::Duration;

#[test]
fn empty_pool_stats_are_zero() {
    let pool = Pool::new();
    let s = pool.stats();
    // PoolStats is #[non_exhaustive]; build the expected via Default (all zero)
    // and set the one non-zero field.
    let mut expected = PoolStats::default();
    expected.max_connections = DEFAULT_MAX_CONNECTIONS;
    assert_eq!(s, expected);
    assert!(pool.is_empty());
    assert_eq!(pool.len(), 0);
}

#[test]
fn with_limits_reflects_in_stats() {
    let pool = Pool::with_limits(Duration::from_secs(10), 4, 6);
    let s = pool.stats();
    assert_eq!(s.max_connections, 4);
    assert_eq!(s.entries, 0);
}

#[test]
fn stats_snapshot_is_copy_and_comparable() {
    let pool = Pool::new();
    let a = pool.stats();
    let b = pool.stats();
    // Snapshots should be copyable values, not tied to the pool's
    // lifetime — so operators can pass them around, log them, etc.
    assert_eq!(a, b);
    let _c = a; // Copy semantic
    let _d = a;
}

#[test]
fn default_constants_are_sane() {
    // Bumped to 2048 for session-persistent pooled workloads that keep one
    // pool entry per (host, proxy) pair; idle timeout matches Chrome's
    // kUsedIdleSocketTimeout (5 min). See pool::pool docs.
    assert_eq!(DEFAULT_MAX_CONNECTIONS, 2048);
    assert_eq!(DEFAULT_IDLE_TIMEOUT, Duration::from_secs(300));
}
