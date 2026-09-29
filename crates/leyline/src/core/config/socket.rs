use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use crate::tls::HappyEyeballsConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SocketConfig {
    pub(crate) local_address: Option<IpAddr>,
    pub(crate) local_ipv4: Option<Ipv4Addr>,
    pub(crate) local_ipv6: Option<Ipv6Addr>,
    pub(crate) tcp_nodelay: Option<bool>,
    pub(crate) tcp_keepalive: Option<Duration>,
    pub(crate) tcp_keepalive_interval: Option<Duration>,
    pub(crate) tcp_keepalive_retries: Option<u32>,
    pub(crate) tcp_user_timeout: Option<Duration>,
    pub(crate) send_buffer_size: Option<usize>,
    pub(crate) recv_buffer_size: Option<usize>,
    pub(crate) strict: bool,
    pub(crate) happy_eyeballs: Option<HappyEyeballsConfig>,
}

impl Default for SocketConfig {
    fn default() -> Self {
        Self {
            local_address: None,
            local_ipv4: None,
            local_ipv6: None,
            tcp_nodelay: None,
            tcp_keepalive: Some(Duration::from_secs(60)),
            tcp_keepalive_interval: Some(Duration::from_secs(30)),
            tcp_keepalive_retries: Some(3),
            tcp_user_timeout: None,
            send_buffer_size: None,
            recv_buffer_size: None,
            strict: false,
            happy_eyeballs: None,
        }
    }
}

impl SocketConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn local_address(mut self, addr: impl Into<Option<IpAddr>>) -> Self {
        self.local_address = addr.into();
        self
    }

    pub fn local_ipv4(mut self, addr: impl Into<Option<Ipv4Addr>>) -> Self {
        self.local_ipv4 = addr.into();
        self
    }

    pub fn local_ipv6(mut self, addr: impl Into<Option<Ipv6Addr>>) -> Self {
        self.local_ipv6 = addr.into();
        self
    }

    pub fn tcp_nodelay(mut self, on: impl Into<Option<bool>>) -> Self {
        self.tcp_nodelay = on.into();
        self
    }

    pub fn tcp_keepalive(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.tcp_keepalive = d.into();
        self
    }

    pub fn tcp_keepalive_interval(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.tcp_keepalive_interval = d.into();
        self
    }

    pub fn tcp_keepalive_retries(mut self, n: impl Into<Option<u32>>) -> Self {
        self.tcp_keepalive_retries = n.into();
        self
    }

    pub fn tcp_user_timeout(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.tcp_user_timeout = d.into();
        self
    }

    pub fn send_buffer_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.send_buffer_size = n.into();
        self
    }

    pub fn recv_buffer_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.recv_buffer_size = n.into();
        self
    }

    pub fn strict(mut self, on: bool) -> Self {
        self.strict = on;
        self
    }

    pub fn happy_eyeballs(mut self, config: impl Into<Option<HappyEyeballsConfig>>) -> Self {
        self.happy_eyeballs = config.into();
        self
    }
}
