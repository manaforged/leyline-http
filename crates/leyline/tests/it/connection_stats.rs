use leyline::pool::{ConnectionStats, DEFAULT_IDLE_TIMEOUT, DEFAULT_MAX_CONNECTIONS, Pool};
use std::time::Duration;

#[test]
fn empty_connection_stats_are_zero() {
    let pool = Pool::new();
    let s = pool.stats();
    let expected = ConnectionStats {
        max_connections: DEFAULT_MAX_CONNECTIONS,
        ..ConnectionStats::default()
    };
    assert_eq!(s, expected);
}

#[test]
fn with_limits_reflects_in_stats() {
    let pool = Pool::with_limits(Duration::from_secs(10), 4, 6);
    let s = pool.stats();
    assert_eq!(s.max_connections, 4);
    assert_eq!(s.entries, 0);
}

#[test]
fn default_constants_are_sane() {
    assert_eq!(DEFAULT_MAX_CONNECTIONS, 2048);
    assert_eq!(DEFAULT_IDLE_TIMEOUT, Duration::from_secs(300));
}
