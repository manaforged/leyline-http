use crate::core::SocketConfig;
use crate::tcp::TcpProfile;
use tokio::net::TcpStream;

pub(crate) async fn connect_one(
    sock_addr: std::net::SocketAddr,
    tcp_profile: &TcpProfile,
    socket_config: &SocketConfig,
) -> Result<TcpStream, std::io::Error> {
    let is_v6 = matches!(sock_addr, std::net::SocketAddr::V6(_));
    let domain = if is_v6 {
        socket2::Domain::IPV6
    } else {
        socket2::Domain::IPV4
    };
    let socket = socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))?;
    bind_local_address(&socket, sock_addr, socket_config)?;
    tcp_profile.apply(&socket, is_v6);
    apply_socket_config(&socket, socket_config)?;
    socket.set_nonblocking(true)?;

    match socket.connect(&sock_addr.into()) {
        Ok(()) => {}
        Err(e) if nonblocking_connect_started(&e) => {}
        Err(e) => return Err(e),
    }
    let std_stream: std::net::TcpStream = socket.into();
    let tcp_stream = TcpStream::from_std(std_stream)?;

    tcp_stream.writable().await?;
    if let Some(e) = tcp_stream.take_error()? {
        return Err(e);
    }
    Ok(tcp_stream)
}

fn bind_local_address(
    socket: &socket2::Socket,
    peer: std::net::SocketAddr,
    config: &SocketConfig,
) -> Result<(), std::io::Error> {
    let local_ip = match (
        peer,
        config.local_address,
        config.local_ipv4,
        config.local_ipv6,
    ) {
        (_, Some(ip), _, _) => Some(ip),
        (std::net::SocketAddr::V4(_), None, Some(ip), _) => Some(std::net::IpAddr::V4(ip)),
        (std::net::SocketAddr::V6(_), None, _, Some(ip)) => Some(std::net::IpAddr::V6(ip)),
        _ => None,
    };
    if let Some(ip) = local_ip {
        socket.bind(&std::net::SocketAddr::new(ip, 0).into())?;
    }
    Ok(())
}

fn apply_socket_config(
    socket: &socket2::Socket,
    config: &SocketConfig,
) -> Result<(), std::io::Error> {
    if let Some(enabled) = config.tcp_nodelay {
        socket.set_nodelay(enabled)?;
    }
    if let Some(size) = config.send_buffer_size {
        socket.set_send_buffer_size(size)?;
    }
    if let Some(size) = config.recv_buffer_size {
        socket.set_recv_buffer_size(size)?;
    }
    if config.tcp_keepalive.is_some()
        || config.tcp_keepalive_interval.is_some()
        || config.tcp_keepalive_retries.is_some()
    {
        let mut keepalive = socket2::TcpKeepalive::new();
        if let Some(time) = config.tcp_keepalive {
            keepalive = keepalive.with_time(time);
        }
        if let Some(interval) = config.tcp_keepalive_interval {
            keepalive = keepalive.with_interval(interval);
        }
        #[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
        if let Some(retries) = config.tcp_keepalive_retries {
            keepalive = keepalive.with_retries(retries);
        }
        #[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
        if config.tcp_keepalive_retries.is_some() {
            tracing::debug!(
                target: "leyline::socket",
                "tcp_keepalive_retries unsupported on this platform; using OS default"
            );
        }
        socket.set_tcp_keepalive(&keepalive)?;
    }
    if config.tcp_user_timeout.is_some() {
        unsupported_socket_option(config.strict, "tcp_user_timeout")?;
    }
    if config.interface.is_some() {
        unsupported_socket_option(config.strict, "interface")?;
    }
    Ok(())
}

fn unsupported_socket_option(strict: bool, name: &str) -> Result<(), std::io::Error> {
    if strict {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!("{name} is not supported on this platform/build"),
        ))
    } else {
        tracing::warn!(target: "leyline::socket", option = name, "socket option not supported");
        Ok(())
    }
}

fn nonblocking_connect_started(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::WouldBlock
        || error.raw_os_error() == Some(libc::EINPROGRESS)
        || windows_nonblocking_connect_started(error)
}

#[cfg(windows)]
fn windows_nonblocking_connect_started(error: &std::io::Error) -> bool {
    const WSAEWOULDBLOCK: i32 = 10035;
    const WSAEINPROGRESS: i32 = 10036;
    matches!(
        error.raw_os_error(),
        Some(WSAEWOULDBLOCK) | Some(WSAEINPROGRESS)
    )
}

#[cfg(not(windows))]
fn windows_nonblocking_connect_started(_error: &std::io::Error) -> bool {
    false
}

#[cfg(test)]
mod tests;
