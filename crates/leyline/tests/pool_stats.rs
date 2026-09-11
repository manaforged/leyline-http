use leyline::pool::{DEFAULT_IDLE_TIMEOUT, DEFAULT_MAX_CONNECTIONS, Pool, PoolStats};
use std::time::Duration;

#[test]
fn empty_pool_stats_are_zero() {
    let pool = Pool::new();
    let s = pool.stats();
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
    assert_eq!(a, b);
    let _c = a;
    let _d = a;
}

#[test]
fn default_constants_are_sane() {
    assert_eq!(DEFAULT_MAX_CONNECTIONS, 2048);
    assert_eq!(DEFAULT_IDLE_TIMEOUT, Duration::from_secs(300));
}
