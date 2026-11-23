use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
#[cfg(unix)]
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use socket2::{Socket, TcpKeepalive};

use super::HappyEyeballs;
use crate::profile::TcpProfile;

mod platform;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) struct SocketOptions {
    pub(crate) local_ipv4: Option<Ipv4Addr>,
    pub(crate) local_ipv6: Option<Ipv6Addr>,
    pub(crate) interface: Option<String>,
    #[cfg(unix)]
    pub(crate) unix_socket: Option<PathBuf>,
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

impl SocketOptions {
    pub(crate) fn invalid_interface_name(name: &str) -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid network interface name `{name}`"),
        )
    }

    pub(crate) fn validate_interface_name(name: &str) -> io::Result<()> {
        if name.is_empty() || name.as_bytes().contains(&0) {
            return Err(Self::invalid_interface_name(name));
        }
        Ok(())
    }

    pub(crate) fn is_interface_config_error(&self, err: &io::Error) -> bool {
        (self.strict && err.kind() == io::ErrorKind::Unsupported)
            || (self.interface.is_some()
                && matches!(
                    err.kind(),
                    io::ErrorKind::InvalidInput
                        | io::ErrorKind::NotFound
                        | io::ErrorKind::Unsupported
                ))
    }

    pub(crate) fn map_io_error<E>(
        &self,
        err: io::Error,
        config: impl FnOnce(io::Error) -> E,
        other: impl FnOnce(io::Error) -> E,
    ) -> E {
        if self.is_interface_config_error(&err) {
            config(err)
        } else {
            other(err)
        }
    }
}

impl Default for SocketOptions {
    fn default() -> Self {
        Self {
            local_ipv4: None,
            local_ipv6: None,
            interface: None,
            #[cfg(unix)]
            unix_socket: None,
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

/// What a failed socket option does to the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Policy {
    Warn,
    Fail,
}

const PROFILE: Policy = Policy::Warn;
const OPTIONS: Policy = Policy::Fail;

impl Policy {
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn unsupported(strict: bool) -> Self {
        if strict { Self::Fail } else { Self::Warn }
    }

    fn settle(self, option: &'static str, result: io::Result<()>) -> io::Result<()> {
        match (self, result) {
            (Self::Warn, Err(e)) => {
                warn_once(option, &e);
                Ok(())
            }
            (_, result) => result,
        }
    }
}

static LOGGED: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

fn warn_once(option: &'static str, err: &io::Error) {
    let mut logged = crate::util::lock(&LOGGED);
    if !logged.contains(&option) {
        logged.push(option);
        tracing::warn!(target: "leyline::socket", option, error = %err, "socket option not applied");
    }
}

fn apply_profile(
    socket: &Socket,
    profile: &TcpProfile,
    options: &SocketOptions,
    is_v6: bool,
) -> io::Result<()> {
    if profile.ttl > 0 {
        let (option, result) = if is_v6 {
            ("IPV6_UNICAST_HOPS", socket.set_unicast_hops_v6(profile.ttl))
        } else {
            ("IP_TTL", socket.set_ttl_v4(profile.ttl))
        };
        PROFILE.settle(option, result)?;
    }
    match (options.tcp_nodelay, profile.no_delay) {
        (Some(on), _) => OPTIONS.settle("TCP_NODELAY", socket.set_tcp_nodelay(on))?,
        (None, true) => PROFILE.settle("TCP_NODELAY", socket.set_tcp_nodelay(true))?,
        (None, false) => {}
    }
    platform::apply(socket, profile, is_v6)
}

/// Binds and configures a new socket for `peer` from the TCP profile and the session's socket options.
pub(super) fn configure(
    socket: &Socket,
    peer: SocketAddr,
    profile: &TcpProfile,
    options: &SocketOptions,
) -> io::Result<()> {
    let is_v6 = peer.is_ipv6();
    bind_interface(socket, is_v6, options)?;
    bind_local(socket, peer, options)?;
    apply_profile(socket, profile, options, is_v6)?;
    if let Some(size) = options.send_buffer_size {
        OPTIONS.settle("SO_SNDBUF", socket.set_send_buffer_size(size))?;
    }
    if let Some(size) = options.recv_buffer_size {
        OPTIONS.settle("SO_RCVBUF", socket.set_recv_buffer_size(size))?;
    }
    if let Some(keepalive) = keepalive(options) {
        OPTIONS.settle("SO_KEEPALIVE", socket.set_tcp_keepalive(&keepalive))?;
    }
    user_timeout(socket, options)
}

fn local_ip_for(peer: SocketAddr, options: &SocketOptions) -> Option<IpAddr> {
    match peer {
        SocketAddr::V4(_) => options.local_ipv4.map(IpAddr::V4),
        SocketAddr::V6(_) => options.local_ipv6.map(IpAddr::V6),
    }
}

pub(super) fn socket_domain(peer: SocketAddr) -> socket2::Domain {
    if peer.is_ipv6() {
        socket2::Domain::IPV6
    } else {
        socket2::Domain::IPV4
    }
}

#[cfg(feature = "http3")]
pub(crate) fn bind_udp(
    peer: SocketAddr,
    options: &SocketOptions,
) -> io::Result<tokio::net::UdpSocket> {
    let is_v6 = peer.is_ipv6();
    let socket = Socket::new(
        socket_domain(peer),
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;
    bind_interface(&socket, is_v6, options)?;
    let local = local_ip_for(peer, options).map_or_else(
        || crate::util::unspecified_for(peer),
        |ip| SocketAddr::new(ip, 0),
    );
    socket.bind(&local.into())?;
    socket.set_nonblocking(true)?;
    tokio::net::UdpSocket::from_std(socket.into())
}

fn bind_interface(socket: &Socket, is_v6: bool, options: &SocketOptions) -> io::Result<()> {
    let Some(name) = options.interface.as_deref() else {
        return Ok(());
    };
    platform::bind_interface(socket, name, is_v6)
}

fn bind_local(socket: &Socket, peer: SocketAddr, options: &SocketOptions) -> io::Result<()> {
    if let Some(ip) = local_ip_for(peer, options) {
        socket.bind(&SocketAddr::new(ip, 0).into())?;
    }
    Ok(())
}

fn keepalive(options: &SocketOptions) -> Option<TcpKeepalive> {
    if options.tcp_keepalive.is_none()
        && options.tcp_keepalive_interval.is_none()
        && options.tcp_keepalive_retries.is_none()
    {
        return None;
    }
    let mut keepalive = TcpKeepalive::new();
    if let Some(time) = options.tcp_keepalive {
        keepalive = keepalive.with_time(time);
    }
    if let Some(interval) = options.tcp_keepalive_interval {
        keepalive = keepalive.with_interval(interval);
    }
    #[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
    if let Some(retries) = options.tcp_keepalive_retries {
        keepalive = keepalive.with_retries(retries);
    }
    #[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
    if options.tcp_keepalive_retries.is_some() {
        tracing::debug!(
            target: "leyline::socket",
            "tcp_keepalive_retries unsupported on this platform; using OS default"
        );
    }
    Some(keepalive)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn user_timeout(socket: &Socket, options: &SocketOptions) -> io::Result<()> {
    match options.tcp_user_timeout {
        Some(timeout) => OPTIONS.settle(
            "TCP_USER_TIMEOUT",
            socket.set_tcp_user_timeout(Some(timeout)),
        ),
        None => Ok(()),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn user_timeout(_socket: &Socket, options: &SocketOptions) -> io::Result<()> {
    const NAME: &str = "tcp_user_timeout";
    if options.tcp_user_timeout.is_none() {
        return Ok(());
    }
    let unsupported = io::Error::new(
        io::ErrorKind::Unsupported,
        format!("{NAME} is not supported on this platform/build"),
    );
    Policy::unsupported(options.strict).settle(NAME, Err(unsupported))
}

#[cfg(test)]
mod tests;
