use super::nonblocking_connect_started;

#[test]
fn accepts_would_block_kind() {
    let err = std::io::Error::from(std::io::ErrorKind::WouldBlock);
    assert!(nonblocking_connect_started(&err));
}

#[test]
fn rejects_unrelated_connect_error() {
    let err = std::io::Error::from(std::io::ErrorKind::ConnectionRefused);
    assert!(!nonblocking_connect_started(&err));
}

#[cfg(unix)]
#[test]
fn accepts_unix_einprogress() {
    let err = std::io::Error::from_raw_os_error(libc::EINPROGRESS);
    assert!(nonblocking_connect_started(&err));
}

#[cfg(windows)]
#[test]
fn accepts_windows_wsaewouldblock() {
    let err = std::io::Error::from_raw_os_error(10035);
    assert!(nonblocking_connect_started(&err));
}

#[cfg(windows)]
#[test]
fn accepts_windows_wsaeinprogress() {
    let err = std::io::Error::from_raw_os_error(10036);
    assert!(nonblocking_connect_started(&err));
}

struct Fixed(Vec<std::net::SocketAddr>);

impl crate::net::resolver::Resolver for Fixed {
    fn resolve<'a>(
        &'a self,
        _host: &'a str,
        _port: u16,
    ) -> crate::net::resolver::ResolveFuture<'a> {
        let addrs = self.0.clone();
        Box::pin(async move { Ok(addrs) })
    }
}

#[tokio::test]
async fn new_connections_spread_across_the_addresses_of_a_host() {
    let mut listeners = Vec::new();
    for _ in 0..3 {
        listeners.push(tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap());
    }
    let addrs: Vec<_> = listeners.iter().map(|l| l.local_addr().unwrap()).collect();
    let dialer = super::Dialer::new(crate::Platform::Linux.tcp_profile())
        .with_resolver(std::sync::Arc::new(Fixed(addrs.clone())));
    let mut reached = Vec::new();
    for _ in 0..6 {
        let stream = dialer.dial("spread.test", 443).await.expect("dial");
        reached.push(stream.peer_addr().unwrap());
    }
    for addr in addrs {
        assert_eq!(
            reached.iter().filter(|r| **r == addr).count(),
            2,
            "{reached:?}"
        );
    }
}

#[tokio::test]
async fn an_address_that_keeps_refusing_moves_to_the_back() {
    let live = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead = {
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        closed.local_addr().unwrap()
    };
    let live_addr = live.local_addr().unwrap();
    let dialer = super::Dialer::new(crate::Platform::Linux.tcp_profile())
        .with_resolver(std::sync::Arc::new(Fixed(vec![dead, live_addr])));
    for _ in 0..10 {
        let stream = dialer.dial("outlier.test", 443).await.expect("dial");
        assert_eq!(stream.peer_addr().unwrap(), live_addr);
    }
    assert_eq!(
        dialer.outliers.order(vec![dead, live_addr]),
        vec![live_addr, dead]
    );
}

#[tokio::test]
async fn dns_timeout_and_config_errors_classify_through_net_error() {
    struct Failing(std::io::ErrorKind);
    impl crate::net::resolver::Resolver for Failing {
        fn resolve<'a>(
            &'a self,
            _host: &'a str,
            _port: u16,
        ) -> crate::net::resolver::ResolveFuture<'a> {
            let kind = self.0;
            Box::pin(async move { Err(std::io::Error::new(kind, "synthetic dns failure")) })
        }
    }

    let timeout_dialer = super::Dialer::new(crate::Platform::Linux.tcp_profile())
        .with_resolver(std::sync::Arc::new(Failing(std::io::ErrorKind::TimedOut)));
    let err = timeout_dialer.dial("timeout.test", 443).await.unwrap_err();
    assert_eq!(err.kind(), crate::error::Kind::Timeout);

    let config_dialer = super::Dialer::new(crate::Platform::Linux.tcp_profile()).with_resolver(
        std::sync::Arc::new(Failing(std::io::ErrorKind::InvalidInput)),
    );
    let err = config_dialer.dial("config.test", 443).await.unwrap_err();
    assert_eq!(err.kind(), crate::error::Kind::Config);

    let other_dialer = super::Dialer::new(crate::Platform::Linux.tcp_profile())
        .with_resolver(std::sync::Arc::new(Failing(std::io::ErrorKind::Other)));
    let err = other_dialer.dial("other.test", 443).await.unwrap_err();
    assert_eq!(err.kind(), crate::error::Kind::Dns);

    let tls_io = std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        leyline_bssl::error::ErrorStack::get(),
    );
    let proxy_tls = crate::error::Error::from(crate::net::NetError::proxy_io(tls_io));
    assert_eq!(proxy_tls.kind(), crate::error::Kind::Proxy);
    assert!(
        !proxy_tls.is_retryable(),
        "TLS alert on HTTPS proxy CONNECT must not be retryable"
    );
}
