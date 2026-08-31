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
