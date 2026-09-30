use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PoolConfig {
    pub(crate) idle_timeout: Duration,
    pub(crate) max_connections: usize,
    pub(crate) max_h1_conns_per_host: usize,
    pub(crate) keepalive: bool,
    pub(crate) h2_ping_after_idle: Option<Duration>,
    pub(crate) h2_ping_timeout: Duration,
    #[cfg(feature = "http3")]
    pub(crate) max_alt_svc_origins: usize,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            idle_timeout: crate::pool::DEFAULT_IDLE_TIMEOUT,
            max_connections: crate::pool::DEFAULT_MAX_CONNECTIONS,
            max_h1_conns_per_host: crate::pool::DEFAULT_MAX_H1_CONNS_PER_HOST,
            keepalive: true,
            h2_ping_after_idle: crate::pool::DEFAULT_H2_PING_AFTER_IDLE,
            h2_ping_timeout: crate::pool::DEFAULT_H2_PING_TIMEOUT,
            #[cfg(feature = "http3")]
            max_alt_svc_origins: crate::pool::DEFAULT_MAX_ALT_SVC_ORIGINS,
        }
    }
}

impl PoolConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn idle_timeout(mut self, d: Duration) -> Self {
        self.idle_timeout = d;
        self
    }

    pub fn max_connections(mut self, n: usize) -> Self {
        self.max_connections = n;
        self
    }

    pub fn max_h1_conns_per_host(mut self, n: usize) -> Self {
        self.max_h1_conns_per_host = n;
        self
    }

    pub fn keepalive(mut self, on: bool) -> Self {
        self.keepalive = on;
        self
    }

    pub fn h2_ping_after_idle(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.h2_ping_after_idle = d.into();
        self
    }

    pub fn h2_ping_timeout(mut self, d: Duration) -> Self {
        self.h2_ping_timeout = d;
        self
    }
}
