use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::h2::client::H2Client;

use super::pool::Pool;
use super::types::{Counter, PoolKey, TlsInfo};

pub const DEFAULT_H2_PING_AFTER_IDLE: Option<Duration> = Some(Duration::from_secs(10));

pub const DEFAULT_H2_PING_TIMEOUT: Duration = Duration::from_secs(2);

impl Pool {
    fn idle_for(&self, key: &PoolKey) -> Option<Duration> {
        let map = crate::util::lock(&self.inner);
        map.get(key)
            .map(|entry| Instant::now().saturating_duration_since(entry.last_use()))
    }
}

pub(crate) async fn checkout_live_h2(
    pool: &Arc<Pool>,
    key: &PoolKey,
) -> Option<(H2Client, TlsInfo)> {
    let idle = pool.idle_for(key);
    let (handle, tls) = pool.checkout_mux::<H2Client>(key)?;
    let Some(after_idle) = pool.config.h2_ping_after_idle else {
        return Some((handle, tls));
    };
    if !handle.take_unanswered_timeout() && idle.is_none_or(|idle| idle < after_idle) {
        return Some((handle, tls));
    }
    match tokio::time::timeout(pool.config.h2_ping_timeout, handle.ping()).await {
        Ok(Ok(())) => Some((handle, tls)),
        outcome => {
            tracing::info!(
                target: "leyline::pool",
                host = %key.host,
                port = key.port,
                proxied = key.proxy.is_some(),
                timed_out = outcome.is_err(),
                "pool ping failed -- dropping idle h2 connection, opening fresh"
            );
            pool.invalidate(key);
            pool.counters.add(Counter::H2PingFailures, 1);
            None
        }
    }
}
