use super::Pool;
use crate::pool::types::{PoolKey, PooledConn, TlsInfo, Transport};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

fn h1_key(host: &str) -> PoolKey {
    PoolKey {
        scheme: "http".to_string(),
        host: host.to_string(),
        port: 80,
        proxy: None,
        transport: Transport::Tcp,
    }
}

fn insert_empty_h1(pool: &Pool, host: &str, last_use: Instant) {
    pool.inner.lock().unwrap_or_else(|e| e.into_inner()).insert(
        h1_key(host),
        PooledConn::H1 {
            idle: VecDeque::new(),
            last_use,
            tls: TlsInfo::default(),
        },
    );
}

#[test]
fn evict_idle_drops_abandoned_empty_h1_entry() {
    let pool = Pool::with_limits(Duration::from_millis(20), 2048, 6);
    let stale = Instant::now()
        .checked_sub(Duration::from_millis(40))
        .expect("monotonic clock is >40ms past its epoch");
    insert_empty_h1(&pool, "abandoned", stale);

    pool.evict_idle();

    assert_eq!(
        pool.len(),
        0,
        "an empty H1 entry idle past the timeout must be reaped, not linger until LRU"
    );
}

#[test]
fn evict_idle_keeps_empty_h1_entry_with_live_checkouts() {
    let pool = Pool::with_limits(Duration::from_secs(300), 2048, 6);
    insert_empty_h1(&pool, "in-flight", Instant::now());

    pool.evict_idle();

    assert_eq!(
        pool.len(),
        1,
        "an empty H1 entry touched within the idle window (checkouts in flight) must survive"
    );
}

#[cfg(feature = "http3")]
#[test]
fn alt_svc_marks_h3_origin() {
    let pool = Pool::new();
    assert!(!pool.knows_h3("example.com", 443));
    pool.note_alt_svc("example.com", 443, "h2=\":443\"; ma=86400");
    assert!(!pool.knows_h3("example.com", 443));
    pool.note_alt_svc("example.com", 443, "h3=\":443\"; ma=86400, h3-29=\":443\"");
    assert!(pool.knows_h3("example.com", 443));
    assert!(!pool.knows_h3("example.com", 8443));
    pool.note_alt_svc("other.example", 443, "h3=\":8443\"");
    assert!(!pool.knows_h3("other.example", 443));
    pool.note_alt_svc("cross.example", 443, "h3=\"elsewhere.example:443\"");
    assert!(!pool.knows_h3("cross.example", 443));
    pool.note_alt_svc("same.example", 443, "h3=\"same.example:443\"");
    assert!(pool.knows_h3("same.example", 443));
}

#[test]
fn alpn_h1_memory_is_per_origin_and_proxy() {
    let pool = Pool::new();
    assert!(!pool.is_h1_only("example.com", 443, None));
    pool.note_h1_only("example.com", 443, None);
    assert!(pool.is_h1_only("example.com", 443, None));
    assert!(!pool.is_h1_only("example.com", 443, Some("http://proxy:1")));
    assert!(!pool.is_h1_only("example.com", 8443, None));
}
