use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::h2::client::H2Client;

use super::pool::Pool;
use super::types::{PoolKey, TlsInfo};

pub const DEFAULT_H2_PING_AFTER_IDLE: Option<Duration> = Some(Duration::from_secs(10));

pub const DEFAULT_H2_PING_TIMEOUT: Duration = Duration::from_secs(2);

impl Pool {
    #[must_use]
    pub fn with_h2_ping(mut self, after_idle: Option<Duration>, timeout: Duration) -> Self {
        self.h2_ping_after_idle = after_idle;
        self.h2_ping_timeout = timeout;
        self
    }

    fn idle_for(&self, key: &PoolKey) -> Option<Duration> {
        let map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        map.get(key)
            .map(|entry| Instant::now().saturating_duration_since(entry.last_use()))
    }
}

pub(crate) async fn checkout_live_h2(
    pool: &Arc<Pool>,
    key: &PoolKey,
) -> Option<(H2Client, TlsInfo)> {
    let idle = pool.idle_for(key);
    let (handle, tls) = pool.checkout_h2(key)?;
    let Some(after_idle) = pool.h2_ping_after_idle else {
        return Some((handle, tls));
    };
    if idle.is_none_or(|idle| idle < after_idle) {
        return Some((handle, tls));
    }
    match tokio::time::timeout(pool.h2_ping_timeout, handle.ping()).await {
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
            pool.counters
                .h2_ping_failures
                .fetch_add(1, Ordering::Relaxed);
            None
        }
    }
}
