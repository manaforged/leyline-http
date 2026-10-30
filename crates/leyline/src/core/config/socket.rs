use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use crate::tls::HappyEyeballs;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) struct SocketOptions {
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
    pub(crate) happy_eyeballs: Option<HappyEyeballs>,
}

impl Default for SocketOptions {
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
