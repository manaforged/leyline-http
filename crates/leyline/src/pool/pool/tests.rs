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

// Regression for the lingering-empty-H1-entry leak. An entry whose deque
// emptied because its connections were all checked out and then died
// mid-request (never returned via `return_h1`) is not idle-swept — `before == idle.len()` held forever for a `0 == 0` empty deque —
// and only left the pool on LRU eviction. Now the idle sweep drops it once
// its `last_use` ages past the idle timeout (no request has borrowed it).
#[test]
fn evict_idle_drops_abandoned_empty_h1_entry() {
    let pool = Pool::with_limits(Duration::from_millis(20), 2048, 6);
    // last_use well past the 20ms idle timeout → no live borrower.
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

// The counterpart the fix must NOT break: an entry that is empty only
// because every connection is currently checked out (in-flight) has a
// recent `last_use`, so the sweep keeps it for the returning requests to
// reuse rather than churning a drop + recreate on every sweep.
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
