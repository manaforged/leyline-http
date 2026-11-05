use std::sync::Arc;
use std::time::Duration;

use tokio::sync::oneshot;

use crate::h2::client::H2Client;

use super::pool::Pool;
use super::types::{Counter, PoolKey, TlsInfo};

pub const DEFAULT_H2_PING_AFTER_IDLE: Option<Duration> = Some(Duration::from_secs(10));

pub const DEFAULT_H2_PING_TIMEOUT: Duration = Duration::from_secs(2);

/// The ACK of a PING sent ahead of a request on a connection that sat idle.
pub(crate) type Probe = oneshot::Receiver<()>;

pub(crate) async fn checkout_live_h2(
    pool: &Arc<Pool>,
    key: &PoolKey,
    now: std::time::Instant,
) -> Option<(H2Client, TlsInfo, Option<Probe>)> {
    let (handle, tls, idle) = pool.checkout_mux_idle::<H2Client>(key, now)?;
    let Some(after_idle) = pool.config.h2_ping_after_idle else {
        return Some((handle, tls, None));
    };
    let suspect = handle.take_unanswered_timeout();
    if !suspect && idle < after_idle {
        return Some((handle, tls, None));
    }
    let probe = handle.probe(pool.config.h2_ping_timeout).await;
    match probe {
        Ok(probe) if !suspect => Some((handle, tls, Some(probe))),
        Ok(probe) => confirm(pool, key, &handle, probe)
            .await
            .then_some((handle, tls, None)),
        Err(error) => {
            dead(pool, key, &handle, &error);
            None
        }
    }
}

/// Waits for a probe's ACK and drops the connection from the pool when it does not come in time.
pub(crate) async fn confirm(pool: &Pool, key: &PoolKey, conn: &H2Client, probe: Probe) -> bool {
    match tokio::time::timeout(pool.config.h2_ping_timeout, probe).await {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            dead(pool, key, conn, &error);
            false
        }
        Err(error) => {
            dead(pool, key, conn, &error);
            false
        }
    }
}

fn dead(pool: &Pool, key: &PoolKey, conn: &H2Client, error: &dyn std::fmt::Display) {
    tracing::info!(
        target: "leyline::pool",
        host = %key.host,
        port = key.port,
        proxied = key.proxy.is_some(),
        %error,
        "pool ping failed -- dropping idle h2 connection, opening fresh"
    );
    pool.invalidate(key, conn);
    pool.counters.add(Counter::H2PingFailures, 1);
}
