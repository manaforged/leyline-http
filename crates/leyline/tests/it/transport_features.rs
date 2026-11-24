use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, Mutex};

use leyline::testing::{Response, Server};
use leyline::{Kind, Protocol, Proxy, Session, Tls};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[cfg(unix)]
struct TempSocket(std::path::PathBuf);

#[cfg(unix)]
impl TempSocket {
    fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "leyline-uds-{tag}-{}-{id}.sock",
            std::process::id()
        ));
        drop(std::fs::remove_file(&path));
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

#[cfg(unix)]
impl Drop for TempSocket {
    fn drop(&mut self) {
        drop(std::fs::remove_file(&self.0));
    }
}

#[tokio::test]
async fn local_addresses_binds_ipv4_loopback() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen_ip = Arc::new(Mutex::new(None));
    let seen_bg = Arc::clone(&seen_ip);
    tokio::spawn(async move {
        let (mut sock, peer) = listener.accept().await.unwrap();
        *seen_bg.lock().unwrap() = Some(peer.ip());
        let mut buf = [0u8; 1024];
        drop(sock.read(&mut buf).await);
        drop(
            sock.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nbound",
            )
            .await,
        );
    });
    let bind_v4 = if cfg!(target_os = "linux") {
        Ipv4Addr::new(127, 0, 0, 2)
    } else {
        Ipv4Addr::LOCALHOST
    };
    let session = Session::builder()
        .local_address(std::net::IpAddr::V4(bind_v4))
        .local_address(std::net::IpAddr::V6(Ipv6Addr::LOCALHOST))
        .build()
        .unwrap();
    let body = session
        .get(format!("http://{addr}/ok"))
        .text()
        .await
        .unwrap();
    assert_eq!(body, "bound");
    assert_eq!(
        *seen_ip.lock().unwrap(),
        Some(std::net::IpAddr::V4(bind_v4)),
        "server must observe the bound local source IP"
    );
}

#[tokio::test]
async fn interface_rejects_unknown_device() {
    let server = Server::http(|_| Response::new(200).body("never"))
        .await
        .unwrap();
    let session = Session::builder()
        .interface("leyline-no-such-if-99")
        .retry(2)
        .build()
        .unwrap();
    let err = session.get(server.url("/")).send().await.unwrap_err();
    assert_eq!(err.kind(), Kind::Config, "expected Config, got {err:?}");
    assert!(!err.is_retryable(), "invalid interface must not retry");
    #[cfg(feature = "http3")]
    {
        let h3_err = session
            .get("https://127.0.0.1:443/")
            .protocol(leyline::Protocol::Http3)
            .send()
            .await
            .unwrap_err();
        assert_eq!(
            h3_err.kind(),
            Kind::Config,
            "expected HTTP/3 Config, got {h3_err:?}"
        );
        assert!(
            !h3_err.is_retryable(),
            "invalid HTTP/3 interface must not retry"
        );
    }
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn interface_binds_loopback_on_macos() {
    let server = Server::http(|_| Response::new(200).body("lo0"))
        .await
        .unwrap();
    let session = Session::builder().interface("lo0").build().unwrap();
    let body = session.get(server.url("/")).text().await.unwrap();
    assert_eq!(body, "lo0");
}

#[tokio::test]
async fn proxy_headers_sent_on_http_forward_and_connect() {
    let seen_forward = Arc::new(Mutex::new(String::new()));
    let seen_forward_bg = Arc::clone(&seen_forward);
    let forward_proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let forward_addr = forward_proxy.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = forward_proxy.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let n = sock.read(&mut buf).await.unwrap();
        *seen_forward_bg.lock().unwrap() = String::from_utf8_lossy(&buf[..n]).into_owned();
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
    });

    let proxy = Proxy::from(format!("http://u:p@{forward_addr}"))
        .header("x-proxy-session", "sess-42")
        .header("proxy-authorization", "Bearer custom-token");
    assert!(
        !format!("{proxy:?}").contains("custom-token"),
        "proxy header values must be masked in Debug"
    );
    let session = Session::builder().proxy(proxy).build().unwrap();
    let body = session
        .get("http://origin.example/item")
        .text()
        .await
        .unwrap();
    assert_eq!(body, "ok");
    let raw = seen_forward.lock().unwrap().to_ascii_lowercase();
    assert!(raw.contains("x-proxy-session: sess-42\r\n"), "{raw}");
    assert!(
        raw.contains("proxy-authorization: bearer custom-token\r\n"),
        "{raw}"
    );
    assert!(
        !raw.contains("basic "),
        "explicit Proxy-Authorization must override URL userinfo: {raw}"
    );

    let https_origin = Server::https(|_| Response::new(200).body("tunneled"))
        .await
        .unwrap();
    let seen_connect = Arc::new(Mutex::new(String::new()));
    let seen_connect_bg = Arc::clone(&seen_connect);
    let connect_proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let connect_addr = connect_proxy.local_addr().unwrap();
    let origin_addr = https_origin.addr();
    tokio::spawn(async move {
        let (mut client, _) = connect_proxy.accept().await.unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 512];
        loop {
            let n = client.read(&mut tmp).await.unwrap();
            buf.extend_from_slice(&tmp[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        *seen_connect_bg.lock().unwrap() = String::from_utf8_lossy(&buf).into_owned();
        let mut upstream = tokio::net::TcpStream::connect(origin_addr).await.unwrap();
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .unwrap();
        drop(tokio::io::copy_bidirectional(&mut client, &mut upstream).await);
    });

    let proxy = Proxy::new()
        .headers([("x-geo", "us-tx"), ("x-sticky", "abc123")])
        .all(format!("http://{connect_addr}"));
    let session = Session::builder()
        .proxy(proxy)
        .tls(https_origin.tls())
        .build()
        .unwrap();
    let body = session
        .get(https_origin.url("/tunnel"))
        .text()
        .await
        .unwrap();
    assert_eq!(body, "tunneled");
    let raw = seen_connect.lock().unwrap().to_ascii_lowercase();
    assert!(raw.starts_with("connect "), "{raw}");
    assert!(raw.contains("x-geo: us-tx\r\n"), "{raw}");
    assert!(raw.contains("x-sticky: abc123\r\n"), "{raw}");
}

#[test]
fn proxy_rejects_forbidden_and_invalid_headers() {
    for bad in ["host", "content-length", "transfer-encoding"] {
        let err = Session::builder()
            .proxy(Proxy::from("http://127.0.0.1:8080").header(bad, "x"))
            .build()
            .unwrap_err();
        assert_eq!(err.kind(), Kind::Config, "{bad}: {err:?}");
    }
    let err = Session::builder()
        .proxy(Proxy::from("http://127.0.0.1:8080").header("x-bad", "line\r\ninject"))
        .build()
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Config);

    let err = Session::builder()
        .proxy(Proxy::list(["http://127.0.0.1:8080"]).header("host", "forbidden"))
        .build()
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Config);
}

#[tokio::test]
async fn proxy_list_forwards_custom_headers() {
    let seen = Arc::new(Mutex::new(String::new()));
    let seen_bg = Arc::clone(&seen);
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = proxy_listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let n = sock.read(&mut buf).await.unwrap();
        *seen_bg.lock().unwrap() = String::from_utf8_lossy(&buf[..n]).into_owned();
        drop(
            sock.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\npool",
            )
            .await,
        );
    });

    let session = Session::builder()
        .proxy(Proxy::list([format!("http://{proxy_addr}")]).header("x-pool-tag", "slot-1"))
        .build()
        .unwrap();
    let body = session.get("http://origin.example/p").text().await.unwrap();
    assert_eq!(body, "pool");
    let raw = seen.lock().unwrap().to_ascii_lowercase();
    assert!(raw.contains("x-pool-tag: slot-1\r\n"), "{raw}");
}

#[cfg(unix)]
#[tokio::test]
async fn unix_socket_serves_http1_and_reuses_connection() {
    let sock = TempSocket::new("http1");
    let listener = tokio::net::UnixListener::bind(sock.path()).unwrap();
    let accepts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let accepts_bg = Arc::clone(&accepts);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            accepts_bg.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let mut filled = 0;
                loop {
                    let n = match stream.read(&mut buf[filled..]).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => n,
                    };
                    filled += n;
                    while let Some(end) = buf[..filled].windows(4).position(|w| w == b"\r\n\r\n") {
                        let req = String::from_utf8_lossy(&buf[..end]).into_owned();
                        let line = req.lines().next().unwrap_or("");
                        let body = format!("unix:{line}");
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                            body.len()
                        );
                        if stream.write_all(resp.as_bytes()).await.is_err() {
                            return;
                        }
                        buf.copy_within(end + 4..filled, 0);
                        filled -= end + 4;
                    }
                }
            });
        }
    });

    let session = Session::builder().unix_socket(sock.path()).build().unwrap();
    let first = session.get("http://docker.local/v1").text().await.unwrap();
    let second = session.get("http://docker.local/v2").text().await.unwrap();
    assert_eq!(first, "unix:GET /v1 HTTP/1.1");
    assert_eq!(second, "unix:GET /v2 HTTP/1.1");
    assert_eq!(
        accepts.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "keep-alive must reuse the pooled Unix socket connection"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn unix_socket_serves_https_with_tls_verification() {
    let sock = TempSocket::new("https");
    let listener = tokio::net::UnixListener::bind(sock.path()).unwrap();
    let (cert, key) = super::tls_support::self_signed_dns("unix.local");
    let cert_pem = cert.to_pem().unwrap();

    let mut acceptor = leyline_bssl::ssl::SslAcceptor::mozilla_intermediate_v5(
        leyline_bssl::ssl::SslMethod::tls(),
    )
    .unwrap();
    acceptor.set_certificate(&cert).unwrap();
    acceptor.set_private_key(&key).unwrap();
    let acceptor = Arc::new(acceptor.build());

    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let ssl = leyline_bssl::ssl::Ssl::new(acceptor.context()).unwrap();
        let mut tls = leyline_bssl_tokio::SslStreamBuilder::new(ssl, stream)
            .accept()
            .await
            .unwrap();
        let mut buf = [0u8; 2048];
        let _ = tls.read(&mut buf).await.unwrap();
        tls.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nsecure-uds",
        )
        .await
        .unwrap();
        drop(tls.shutdown().await);
    });

    let session = Session::builder()
        .unix_socket(sock.path())
        .tls(Tls::new().ca_pem(cert_pem))
        .build()
        .unwrap();
    let body = session
        .get("https://unix.local/secret")
        .text()
        .await
        .unwrap();
    assert_eq!(body, "secure-uds");
}

#[cfg(unix)]
#[tokio::test]
async fn unix_socket_rejects_proxy_and_http3() {
    let err = Session::builder()
        .unix_socket("/tmp/nonexistent.sock")
        .proxy("http://127.0.0.1:8080")
        .build()
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Config);

    #[cfg(feature = "http3")]
    {
        let err = Session::builder()
            .unix_socket("/tmp/nonexistent.sock")
            .protocol(Protocol::Http3)
            .build()
            .unwrap_err();
        assert_eq!(err.kind(), Kind::Config);

        let session = Session::builder()
            .unix_socket("/tmp/nonexistent.sock")
            .build()
            .unwrap();
        let err = session
            .get("https://unix.local/")
            .protocol(Protocol::Http3)
            .send()
            .await
            .unwrap_err();
        assert_eq!(err.kind(), Kind::Config);
    }
    let session = Session::builder()
        .unix_socket("/tmp/nonexistent.sock")
        .build()
        .unwrap();
    let err = session
        .get("http://unix.local/")
        .proxy("http://127.0.0.1:8080")
        .send()
        .await
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Config);
    let _ = Protocol::Http1;
}

#[tokio::test]
async fn proxy_connect_response_with_bare_lf_does_not_hang() {
    use std::time::Duration;

    let origin = Server::https(|_| Response::new(200).body("through-bare-lf"))
        .await
        .unwrap();
    let origin_addr = origin.addr();

    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut client, _) = proxy.accept().await.unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 256];
        loop {
            let n = client.read(&mut tmp).await.unwrap();
            if n == 0 {
                return;
            }
            buf.extend_from_slice(&tmp[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let mut upstream = tokio::net::TcpStream::connect(origin_addr).await.unwrap();
        client
            .write_all(b"HTTP/1.1 200 Connection Established\nVia: bare-lf\r\n\r\n")
            .await
            .unwrap();
        drop(tokio::io::copy_bidirectional(&mut client, &mut upstream).await);
    });

    let session = Session::builder()
        .proxy(format!("http://{proxy_addr}"))
        .tls(origin.tls())
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let err = tokio::time::timeout(Duration::from_secs(2), session.get(origin.url("/")).send())
        .await
        .expect("bare LF proxy CONNECT must not hang until connect timeout")
        .unwrap_err();
    assert_eq!(
        err.kind(),
        Kind::Proxy,
        "bare LF proxy CONNECT must fail with Kind::Proxy, got {err:?}"
    );
}

#[test]
fn coarse_timeout_recovers_after_unpolled_runtime_drop() {
    use std::time::Duration;

    let session = Session::builder()
        .retry(0)
        .timeout(
            leyline::Timeout::new()
                .total(Duration::from_secs(3))
                .read(Duration::from_secs(30)),
        )
        .build()
        .unwrap();

    let rt1 = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt1.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::select! {
            biased;
            _ = session.get(format!("http://{addr}/cancel")).send() => {}
            _ = std::future::ready(()) => {}
        }
    });
    drop(rt1);

    let rt2 = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt2.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _hold = tokio::spawn(async move {
            let (_sock, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(30)).await;
        });
        let started = std::time::Instant::now();
        let req = tokio::spawn({
            let session = session.clone();
            async move { session.get(format!("http://{addr}/hang")).send().await }
        });
        let err = tokio::time::timeout(Duration::from_secs(5), req)
            .await
            .expect("3s coarse timeout must fire within 5s after unpolled rt1 drop")
            .unwrap()
            .unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "3s coarse timeout took {:?} instead of firing on its own ticker",
            started.elapsed()
        );
        assert_eq!(
            err.kind(),
            Kind::Timeout,
            "coarse ticker must restart after unpolled runtime drop: {err:?}"
        );
    });
}

#[test]
fn coalesced_dns_recovers_after_unpolled_runtime_drop() {
    use std::net::SocketAddr;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    struct CountingResolver {
        target: SocketAddr,
        calls: AtomicUsize,
    }

    impl leyline::Resolver for CountingResolver {
        fn resolve<'a>(
            &'a self,
            _host: &'a str,
            _port: u16,
        ) -> Pin<Box<dyn std::future::Future<Output = std::io::Result<Vec<SocketAddr>>> + Send + 'a>>
        {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let target = self.target;
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok(vec![target])
            })
        }
    }

    let rt_server = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let (addr, _server_guard) = rt_server.block_on(async {
        let server = Server::http(|_| Response::new(200).body("dns-ok"))
            .await
            .unwrap();
        (server.addr(), server)
    });

    let resolver = Arc::new(CountingResolver {
        target: addr,
        calls: AtomicUsize::new(0),
    });
    let session = Session::builder()
        .retry(0)
        .timeout(Duration::from_secs(5))
        .dns_resolver(Arc::clone(&resolver) as Arc<dyn leyline::Resolver>)
        .build()
        .unwrap();

    let url = format!("http://coalesce-drop.test:{}/ok", addr.port());
    let rt1 = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt1.block_on(async {
        tokio::select! {
            biased;
            _ = session.get(&url).send() => {}
            _ = std::future::ready(()) => {}
        }
    });
    drop(rt1);
    assert_eq!(
        resolver.calls.load(Ordering::SeqCst),
        0,
        "unpolled rt1 task must be dropped before running resolver"
    );

    let rt2 = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt2.block_on(async {
        let body = session
            .get(&url)
            .text()
            .await
            .expect("host must not stay poisoned after unpolled DNS runtime drop");
        assert_eq!(body, "dns-ok");
    });
    assert!(
        resolver.calls.load(Ordering::SeqCst) >= 1,
        "rt2 must invoke resolver after rt1 unpolled drop"
    );
    drop(_server_guard);
    drop(rt_server);
}
