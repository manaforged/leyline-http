use std::time::Instant;

use crate::core::config::HostStats;
use crate::core::proxy_pool::ProxyHealth;
use crate::pool::ConnectionStats;

/// Holds a point-in-time copy of a session's pool, host, and proxy counters.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Stats {
    pool: ConnectionStats,
    hosts: Vec<HostStats>,
    proxies: Vec<ProxyHealth>,
}

impl Stats {
    pub(crate) fn new(
        pool: ConnectionStats,
        hosts: Vec<HostStats>,
        proxies: Vec<ProxyHealth>,
    ) -> Self {
        Self {
            pool,
            hosts,
            proxies,
        }
    }

    /// Returns how many entries the connection pool holds.
    #[must_use]
    pub fn entries(&self) -> usize {
        self.pool.entries
    }

    /// Returns the connection pool's connection limit.
    #[must_use]
    pub fn max_connections(&self) -> usize {
        self.pool.max_connections
    }

    /// Returns how many HTTP/2 checkouts reused a connection.
    #[must_use]
    pub fn h2_hits(&self) -> u64 {
        self.pool.h2_hits
    }

    /// Returns how many HTTP/2 checkouts opened a connection.
    #[must_use]
    pub fn h2_misses(&self) -> u64 {
        self.pool.h2_misses
    }

    /// Returns how many HTTP/1 checkouts reused a connection.
    #[must_use]
    pub fn h1_hits(&self) -> u64 {
        self.pool.h1_hits
    }

    /// Returns how many HTTP/1 checkouts opened a connection.
    #[must_use]
    pub fn h1_misses(&self) -> u64 {
        self.pool.h1_misses
    }

    /// Returns how many HTTP/3 checkouts reused a connection.
    #[must_use]
    pub fn h3_hits(&self) -> u64 {
        self.pool.h3_hits
    }

    /// Returns how many HTTP/3 checkouts opened a connection.
    #[must_use]
    pub fn h3_misses(&self) -> u64 {
        self.pool.h3_misses
    }

    /// Returns how many connections the pool closed for idling too long.
    #[must_use]
    pub fn evictions_idle(&self) -> u64 {
        self.pool.evictions_idle
    }

    /// Returns how many connections the pool closed to stay under its limit.
    #[must_use]
    pub fn evictions_lru(&self) -> u64 {
        self.pool.evictions_lru
    }

    /// Returns how many dead connections the pool removed.
    #[must_use]
    pub fn evictions_dead(&self) -> u64 {
        self.pool.evictions_dead
    }

    /// Returns how many idle connections the pool probed before reuse.
    #[must_use]
    pub fn stale_probed(&self) -> u64 {
        self.pool.stale_probed
    }

    /// Returns how many HTTP/2 keepalive pings failed.
    #[must_use]
    pub fn h2_ping_failures(&self) -> u64 {
        self.pool.h2_ping_failures
    }

    /// Returns how many connections the pool installed.
    #[must_use]
    pub fn installs(&self) -> u64 {
        self.pool.installs
    }

    /// Returns how many pooled connections carry a request now.
    #[must_use]
    pub fn busy(&self) -> usize {
        self.pool.busy
    }

    /// Returns how many pooled connections wait for a request now.
    #[must_use]
    pub fn idle(&self) -> usize {
        self.pool.idle
    }

    /// Returns each origin that the host limits track.
    pub fn hosts(&self) -> impl Iterator<Item = &str> {
        self.hosts.iter().map(HostStats::origin)
    }

    /// Returns how many requests to `host` are in flight, 0 for an unknown host.
    #[must_use]
    pub fn in_flight(&self, host: &str) -> usize {
        self.host(host).map_or(0, HostStats::in_flight)
    }

    /// Returns how many requests to `host` wait for a slot, 0 for an unknown host.
    #[must_use]
    pub fn waiting(&self, host: &str) -> usize {
        self.host(host).map_or(0, HostStats::waiting)
    }

    /// Returns the redacted label of each proxy in the session's proxy list.
    pub fn proxies(&self) -> impl Iterator<Item = &str> {
        self.proxies.iter().map(ProxyHealth::proxy)
    }

    /// Returns how many requests hold `proxy` now, 0 for an unknown proxy.
    #[must_use]
    pub fn proxy_in_use(&self, proxy: &str) -> u32 {
        self.proxy(proxy).map_or(0, ProxyHealth::in_use)
    }

    /// Returns the failure count of `proxy`, 0 for an unknown proxy.
    #[must_use]
    pub fn proxy_failures(&self, proxy: &str) -> u32 {
        self.proxy(proxy).map_or(0, ProxyHealth::failures)
    }

    /// Returns when the ban on `proxy` ends, or `None` when it is not banned.
    #[must_use]
    pub fn proxy_banned_until(&self, proxy: &str) -> Option<Instant> {
        self.proxy(proxy).and_then(ProxyHealth::banned_until)
    }

    fn host(&self, host: &str) -> Option<&HostStats> {
        self.hosts.iter().find(|stats| stats.origin() == host)
    }

    fn proxy(&self, proxy: &str) -> Option<&ProxyHealth> {
        self.proxies.iter().find(|health| health.proxy() == proxy)
    }
}
