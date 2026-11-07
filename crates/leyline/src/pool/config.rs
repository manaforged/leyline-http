use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PoolConfig {
    pub(crate) idle_timeout: Duration,
    pub(crate) max_connections: usize,
    pub(crate) max_h1_conns_per_host: usize,
    pub(crate) max_h1_pending_per_host: Option<usize>,
    pub(crate) max_lifetime: Option<Duration>,
    pub(crate) max_h2_conns_per_host: Option<usize>,
    pub(crate) keepalive: bool,
    pub(crate) h2_ping_after_idle: Option<Duration>,
    pub(crate) h2_ping_timeout: Duration,
    pub(crate) blocking: bool,
}

/// How a pooled connection does its socket IO.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Io {
    Async,
    Blocking(Option<Duration>),
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            idle_timeout: crate::pool::DEFAULT_IDLE_TIMEOUT,
            max_connections: crate::pool::DEFAULT_MAX_CONNECTIONS,
            max_h1_conns_per_host: crate::pool::DEFAULT_MAX_H1_CONNS_PER_HOST,
            max_h1_pending_per_host: Some(crate::pool::DEFAULT_MAX_H1_PENDING_PER_HOST),
            max_lifetime: None,
            max_h2_conns_per_host: None,
            keepalive: true,
            h2_ping_after_idle: crate::pool::DEFAULT_H2_PING_AFTER_IDLE,
            h2_ping_timeout: crate::pool::DEFAULT_H2_PING_TIMEOUT,
            blocking: false,
        }
    }
}

impl PoolConfig {
    pub(crate) fn io(&self, wait: Option<Duration>) -> Io {
        if self.blocking {
            Io::Blocking(wait)
        } else {
            Io::Async
        }
    }

    pub(crate) fn h2_conns_per_host(&self, impersonates: bool) -> usize {
        self.max_h2_conns_per_host.unwrap_or(if impersonates {
            1
        } else {
            self.max_h1_conns_per_host
        })
    }
}
