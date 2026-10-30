use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use super::SessionBuilder;
use crate::core::SocketOptions;
use crate::tls::HappyEyeballs;

impl SessionBuilder {
    fn update_socket(mut self, f: impl FnOnce(&mut SocketOptions)) -> Self {
        self.own_pool();
        f(&mut self.socket_config);
        self
    }

    fn update_happy_eyeballs(self, f: impl FnOnce(&mut HappyEyeballs)) -> Self {
        self.update_socket(|s| f(s.happy_eyeballs.get_or_insert_with(HappyEyeballs::default)))
    }

    /// Binds new connections to a local address; the default lets the OS choose.
    pub fn local_address(self, addr: impl Into<Option<IpAddr>>) -> Self {
        self.update_socket(|s| s.local_address = addr.into())
    }

    /// Binds new IPv4 connections to a local address; the default lets the OS choose.
    pub fn local_ipv4(self, addr: impl Into<Option<Ipv4Addr>>) -> Self {
        self.update_socket(|s| s.local_ipv4 = addr.into())
    }

    /// Binds new IPv6 connections to a local address; the default lets the OS choose.
    pub fn local_ipv6(self, addr: impl Into<Option<Ipv6Addr>>) -> Self {
        self.update_socket(|s| s.local_ipv6 = addr.into())
    }

    /// Sets `TCP_NODELAY` on new sockets; the default leaves the OS setting.
    pub fn tcp_nodelay(self, on: impl Into<Option<bool>>) -> Self {
        self.update_socket(|s| s.tcp_nodelay = on.into())
    }

    /// Sets the TCP keepalive idle time, which defaults to 60 seconds.
    pub fn tcp_keepalive(self, d: impl Into<Option<Duration>>) -> Self {
        self.update_socket(|s| s.tcp_keepalive = d.into())
    }

    /// Sets the TCP keepalive probe interval, which defaults to 30 seconds.
    pub fn tcp_keepalive_interval(self, d: impl Into<Option<Duration>>) -> Self {
        self.update_socket(|s| s.tcp_keepalive_interval = d.into())
    }

    /// Sets the TCP keepalive probe count, which defaults to 3.
    pub fn tcp_keepalive_retries(self, n: impl Into<Option<u32>>) -> Self {
        self.update_socket(|s| s.tcp_keepalive_retries = n.into())
    }

    /// Sets `TCP_USER_TIMEOUT` on new sockets; the default leaves the OS setting.
    pub fn tcp_user_timeout(self, d: impl Into<Option<Duration>>) -> Self {
        self.update_socket(|s| s.tcp_user_timeout = d.into())
    }

    /// Sets the socket send buffer size; the default leaves the OS setting.
    pub fn send_buffer_size(self, n: impl Into<Option<usize>>) -> Self {
        self.update_socket(|s| s.send_buffer_size = n.into())
    }

    /// Sets the socket receive buffer size; the default leaves the OS setting.
    pub fn recv_buffer_size(self, n: impl Into<Option<usize>>) -> Self {
        self.update_socket(|s| s.recv_buffer_size = n.into())
    }

    /// Makes an unsupported socket option fail the connection, which is off by default.
    pub fn strict_socket_options(self, on: bool) -> Self {
        self.update_socket(|s| s.strict = on)
    }

    /// Enables Happy Eyeballs with the given delay between attempts, which defaults to 250 milliseconds.
    pub fn happy_eyeballs_delay(self, d: Duration) -> Self {
        self.update_happy_eyeballs(|he| he.resolve_delay = d)
    }

    /// Enables Happy Eyeballs with the given attempt limit, which defaults to 8.
    pub fn happy_eyeballs_attempts(self, n: usize) -> Self {
        self.update_happy_eyeballs(|he| he.attempt_limit = n)
    }
}
