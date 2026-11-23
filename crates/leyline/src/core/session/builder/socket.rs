use std::net::IpAddr;
#[cfg(unix)]
use std::path::Path;
use std::time::Duration;

use super::SessionBuilder;
use crate::net::{HappyEyeballs, SocketOptions};

impl SessionBuilder {
    pub(super) fn with_socket_options(mut self, options: SocketOptions) -> Self {
        self.socket_config = options;
        self
    }

    fn update_socket(mut self, f: impl FnOnce(&mut SocketOptions)) -> Self {
        f(&mut self.socket_config);
        self
    }

    fn update_happy_eyeballs(self, f: impl FnOnce(&mut HappyEyeballs)) -> Self {
        self.update_socket(|s| f(s.happy_eyeballs.get_or_insert_with(HappyEyeballs::default)))
    }

    /// Binds new connections of the address's family to a local address, or clears both families with `None`.
    pub fn local_address(self, addr: impl Into<Option<IpAddr>>) -> Self {
        let addr = addr.into();
        self.update_socket(|s| match addr {
            Some(IpAddr::V4(ip)) => s.local_ipv4 = Some(ip),
            Some(IpAddr::V6(ip)) => s.local_ipv6 = Some(ip),
            None => {
                s.local_ipv4 = None;
                s.local_ipv6 = None;
            }
        })
    }

    /// Binds new sockets to the named network interface (Linux, Android, Fuchsia, and Apple platforms), such as `"eth0"` or `"lo0"`.
    pub fn interface(self, name: impl Into<String>) -> Self {
        let name = name.into();
        self.update_socket(|s| s.interface = Some(name))
    }

    /// Routes all HTTP and HTTPS connections for this session over a Unix domain socket at `path`.
    #[cfg(unix)]
    #[cfg_attr(docsrs, doc(cfg(unix)))]
    pub fn unix_socket(self, path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        self.update_socket(|s| s.unix_socket = Some(path))
    }

    /// Sets `TCP_NODELAY` on new sockets; the default leaves the OS setting.
    pub fn tcp_nodelay(self, on: impl Into<Option<bool>>) -> Self {
        self.update_socket(|s| s.tcp_nodelay = on.into())
    }

    /// Sets the TCP keepalive idle time on new sockets; the default turns keepalive on.
    pub fn tcp_keepalive(self, d: impl Into<Option<Duration>>) -> Self {
        self.update_socket(|s| s.tcp_keepalive = d.into())
    }

    /// Sets the TCP keepalive probe interval on new sockets.
    pub fn tcp_keepalive_interval(self, d: impl Into<Option<Duration>>) -> Self {
        self.update_socket(|s| s.tcp_keepalive_interval = d.into())
    }

    /// Sets the TCP keepalive probe count on new sockets.
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

    /// Enables Happy Eyeballs with the given delay between connection attempts.
    pub fn happy_eyeballs_delay(self, d: Duration) -> Self {
        self.update_happy_eyeballs(|he| he.resolve_delay = d)
    }

    /// Enables Happy Eyeballs with the given limit on connection attempts.
    pub fn happy_eyeballs_attempts(self, n: usize) -> Self {
        self.update_happy_eyeballs(|he| he.attempt_limit = n)
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use std::time::Duration;

    use crate::Session;

    #[test]
    fn socket_setters_store_values_on_socket_config() {
        let builder = Session::builder()
            .local_address(IpAddr::V4(Ipv4Addr::LOCALHOST))
            .local_address(IpAddr::V6(Ipv6Addr::LOCALHOST))
            .interface("lo0")
            .tcp_nodelay(true)
            .tcp_keepalive(Duration::from_secs(15))
            .tcp_keepalive_interval(Duration::from_secs(5))
            .tcp_keepalive_retries(7)
            .tcp_user_timeout(Duration::from_secs(9))
            .send_buffer_size(128 * 1024)
            .recv_buffer_size(256 * 1024)
            .strict_socket_options(true);
        assert_eq!(builder.socket_config.local_ipv4, Some(Ipv4Addr::LOCALHOST));
        assert_eq!(builder.socket_config.local_ipv6, Some(Ipv6Addr::LOCALHOST));
        assert_eq!(builder.socket_config.interface.as_deref(), Some("lo0"));
        assert_eq!(builder.socket_config.tcp_nodelay, Some(true));
        assert_eq!(
            builder.socket_config.tcp_keepalive,
            Some(Duration::from_secs(15))
        );
        assert_eq!(
            builder.socket_config.tcp_keepalive_interval,
            Some(Duration::from_secs(5))
        );
        assert_eq!(builder.socket_config.tcp_keepalive_retries, Some(7));
        assert_eq!(
            builder.socket_config.tcp_user_timeout,
            Some(Duration::from_secs(9))
        );
        assert_eq!(builder.socket_config.send_buffer_size, Some(128 * 1024));
        assert_eq!(builder.socket_config.recv_buffer_size, Some(256 * 1024));
        assert!(builder.socket_config.strict);

        let cleared = builder
            .local_address(None)
            .tcp_keepalive_retries(None)
            .send_buffer_size(None);
        assert_eq!(cleared.socket_config.local_ipv4, None);
        assert_eq!(cleared.socket_config.local_ipv6, None);
        assert_eq!(cleared.socket_config.tcp_keepalive_retries, None);
        assert_eq!(cleared.socket_config.send_buffer_size, None);
    }
}
