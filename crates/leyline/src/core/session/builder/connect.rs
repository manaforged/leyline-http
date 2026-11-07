use std::time::Duration;

use crate::core::Protocol;
use crate::error::{Error, Result};
use crate::net::{Dialer, DnsConfig, SocketOptions};
use crate::pool::Io;
use crate::pool::{Pool, PoolConfig};
use crate::profile::BrowserProfile;
use crate::profile::TcpProfile;
use crate::tls::FingerprintConnector;
use crate::tls::trust::Tls;

use super::SessionBuilder;

/// Every builder setting the connection pool and its connector are built from.
#[derive(PartialEq)]
pub(super) struct PoolInputs<'a> {
    pool: PoolConfig,
    io: Io,
    connect_timeout: Option<Duration>,
    tls: &'a Tls,
    dns: &'a DnsConfig,
    socket: &'a SocketOptions,
    tcp: Option<&'a TcpProfile>,
    protocol: Protocol,
}

impl PoolInputs<'_> {
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
        let dialer = Dialer::new(tcp_profile.clone())
            .with_resolver(self.dns.clone().into_resolver())
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
            socket: &self.socket_config,
            tcp: self.tcp_profile.as_ref(),
            protocol: self.protocol_policy,
        }
    }

    pub(super) fn resolve_tcp_profile(&self) -> TcpProfile {
        self.tcp_profile.clone().unwrap_or_else(|| {
            let mut tcp = self.platform.tcp_profile();
            if !self.impersonates() {
                tcp.mss = 0;
                tcp.window_size = 0;
                tcp.window_scale = 0;
            }
            tcp
        })
    }
}
