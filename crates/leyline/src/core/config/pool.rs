use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PoolConfig {
    pub(crate) idle_timeout: Duration,
    pub(crate) max_connections: usize,
    pub(crate) max_h1_conns_per_host: usize,
    pub(crate) max_h2_conns_per_host: Option<usize>,
    pub(crate) keepalive: bool,
    pub(crate) h2_ping_after_idle: Option<Duration>,
    pub(crate) h2_ping_timeout: Duration,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            idle_timeout: crate::pool::DEFAULT_IDLE_TIMEOUT,
            max_connections: crate::pool::DEFAULT_MAX_CONNECTIONS,
            max_h1_conns_per_host: crate::pool::DEFAULT_MAX_H1_CONNS_PER_HOST,
            max_h2_conns_per_host: None,
            keepalive: true,
            h2_ping_after_idle: crate::pool::DEFAULT_H2_PING_AFTER_IDLE,
            h2_ping_timeout: crate::pool::DEFAULT_H2_PING_TIMEOUT,
        }
    }
}

impl PoolConfig {
    pub(crate) fn resolved(self, impersonates: bool) -> Self {
        let (idle_timeout, max_connections) = if self.keepalive {
            (self.idle_timeout, self.max_connections.max(1))
        } else {
            (Duration::ZERO, 1)
        };
        Self {
            idle_timeout,
            max_connections,
            max_h1_conns_per_host: self.max_h1_conns_per_host.max(1),
            max_h2_conns_per_host: Some(
                self.max_h2_conns_per_host
                    .unwrap_or(if impersonates {
                        1
                    } else {
                        self.max_h1_conns_per_host
                    })
                    .max(1),
            ),
            ..self
        }
    }

    pub(crate) fn h2_lanes(&self) -> u8 {
        u8::try_from(self.max_h2_conns_per_host.unwrap_or(1)).unwrap_or(u8::MAX)
    }
}
