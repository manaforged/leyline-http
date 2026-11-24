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
