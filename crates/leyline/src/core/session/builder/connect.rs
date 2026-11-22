use std::sync::Arc;
use std::time::Duration;

use crate::core::doh::DohResolver;
use crate::core::proxy_pool::ProxyPool;
use crate::core::{Protocol, Proxy, Redirect, Session};
use crate::error::{Error, Result};
use crate::net::{Dialer, DnsConfig, Resolver, SocketOptions};
use crate::pool::Io;
use crate::pool::{Pool, PoolConfig};
use crate::profile::BrowserProfile;
use crate::profile::Ja4t;
use crate::profile::TcpProfile;
use crate::tls::FingerprintConnector;
use crate::tls::trust::Tls;

use super::SessionBuilder;

/// Every builder setting the connection pool and its connector are built from.
pub(super) struct PoolInputs<'a> {
    pool: PoolConfig,
    io: Io,
    connect_timeout: Option<Duration>,
    tls: &'a Tls,
    dns: &'a DnsConfig,
    doh: Option<&'a url::Url>,
    proxy: &'a Proxy,
    proxy_pool: Option<&'a ProxyPool>,
    socket: &'a SocketOptions,
    tcp: Option<&'a TcpProfile>,
    ja4t: Option<&'a Ja4t>,
    protocol: Protocol,
}

impl PartialEq for PoolInputs<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.pool == other.pool
            && self.io == other.io
            && self.connect_timeout == other.connect_timeout
            && self.tls == other.tls
            && self.dns == other.dns
            && self.doh == other.doh
            && (self.doh.is_none()
                || (self.proxy.same_for_doh(other.proxy)
                    && match (self.proxy_pool, other.proxy_pool) {
                        (Some(a), Some(b)) => a.same_for_doh(b),
                        (None, None) => true,
                        _ => false,
                    }))
            && self.socket == other.socket
            && self.tcp == other.tcp
            && self.ja4t == other.ja4t
            && self.protocol == other.protocol
    }
}

impl PoolInputs<'_> {
    fn build_doh(&self, endpoint: &url::Url) -> Result<Arc<dyn Resolver>> {
        let mut trust = self.tls.clone();
        trust.clear_origin_identity();
        let proxy = match self.proxy_pool {
            Some(pool) => Proxy::pooled(pool.for_doh()),
            None => self.proxy.clone(),
        };
        let mut builder = Session::builder()
            .tls(trust)
            .proxy(proxy)
            .cookies(None)
            .cache(None)
            .redirect(Redirect::none());
        let mut socket = self.socket.clone();
        #[cfg(unix)]
        {
            socket.unix_socket = None;
        }
        builder = builder.with_socket_options(socket);
        builder = builder.timeout(
            crate::deadline::Timeout::new()
                .connect(self.connect_timeout)
                .total(self.connect_timeout),
        );
        for (host, addrs) in self.dns.overrides() {
            builder = builder.resolve(host, addrs.iter().copied());
        }
        let client = builder.build()?;
        Ok(Arc::new(DohResolver::new(endpoint.clone(), client)))
    }

    pub(super) fn connector(
        &self,
        profile: &BrowserProfile,
        tcp_profile: &TcpProfile,
    ) -> Result<FingerprintConnector> {
        let tls_trust = if self.tls.accepts_invalid_certs() {
            self.tls.clone().system_roots(false)
        } else {
            self.tls.clone()
        };
        let doh = match self.doh {
            Some(endpoint) => Some(self.build_doh(endpoint)?),
            None => None,
        };
        let dialer = Dialer::new(tcp_profile.clone())
            .with_resolver(self.dns.clone().into_resolver_with(doh))
            .with_socket(self.socket.clone());
        let mut fp =
            FingerprintConnector::with_dialer(profile, dialer, &tls_trust).map_err(Error::from)?;
        if let Some(connect_timeout) = self.connect_timeout {
            fp = fp.with_connect_timeout(connect_timeout);
        }
        Ok(fp)
    }

    pub(super) fn pool(&self, impersonates: bool) -> Pool {
        Pool::from_config(self.pool, self.io, impersonates)
    }
}

impl SessionBuilder {
    pub(super) fn pool_inputs(&self) -> PoolInputs<'_> {
        let pool = self.pool_config;
        PoolInputs {
            pool,
            io: pool.io(self.timeouts.read_limit().or(self.timeouts.total_limit())),
            connect_timeout: self.timeouts.connect_limit(),
            tls: &self.tls_trust,
            dns: &self.dns_config,
            doh: self.doh_endpoint.as_ref(),
            proxy: &self.proxy_config,
            proxy_pool: self.proxy_pool.as_ref(),
            socket: &self.socket_config,
            tcp: self.tcp_profile.as_ref(),
            ja4t: self.ja4t.as_ref(),
            protocol: self.protocol_policy,
        }
    }

    pub(super) fn resolve_tcp_profile(&self) -> TcpProfile {
        let mut tcp = self.tcp_profile.clone().unwrap_or_else(|| {
            let mut tcp = self.platform.tcp_profile();
            if !self.impersonates() {
                tcp.mss = 0;
                tcp.window_size = 0;
                tcp.window_scale = 0;
            }
            tcp
        });
        if let Some(ja4t) = &self.ja4t {
            ja4t.apply(&mut tcp);
        }
        tcp
    }
}
