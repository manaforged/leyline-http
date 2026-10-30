use leyline::Browser;

#[tokio::test]
async fn offline_http_connect_proxy_wire_bytes() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = Vec::with_capacity(512);
        let mut tmp = [0u8; 256];
        loop {
            let n = stream.read(&mut tmp).await.unwrap();
            if n == 0 {
                return Err("client closed before request headers".to_string());
            }
            buf.extend_from_slice(&tmp[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let req = String::from_utf8_lossy(&buf).into_owned();

        if !req.starts_with("CONNECT target.example.com:443 HTTP/1.1\r\n") {
            return Err(format!("bad request line: {req:?}"));
        }
        if !req.contains("Host: target.example.com:443") {
            return Err(format!("missing Host header: {req:?}"));
        }
        if !req.contains("Proxy-Authorization: Basic ") {
            return Err(format!("missing Proxy-Authorization: {req:?}"));
        }

        stream
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .await
            .unwrap();
        drop(stream.shutdown().await);
        Ok(req)
    });

    let proxy_url = format!(
        "http://testuser:testpass@{}:{}",
        proxy_addr.ip(),
        proxy_addr.port()
    );
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .proxy(&proxy_url)
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();

    drop(session.get("https://target.example.com/").send().await);

    let recv = server.await.unwrap();
    match recv {
        Ok(req) => println!(
            "✓ offline HTTP CONNECT proxy wire bytes OK ({} bytes header)",
            req.len()
        ),
        Err(e) => panic!("mock proxy rejected CONNECT request: {e}"),
    }
}

#[cfg(feature = "socks")]
#[tokio::test]
async fn offline_socks5_proxy_wire_bytes() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        let mut hdr = [0u8; 2];
        stream.read_exact(&mut hdr).await.unwrap();
        assert_eq!(hdr[0], 0x05, "socks5 version wrong");
        let n_methods = hdr[1] as usize;
        let mut methods = vec![0u8; n_methods];
        stream.read_exact(&mut methods).await.unwrap();
        assert!(
            methods.contains(&0x02),
            "expected USERNAME/PASSWORD (0x02) in offered auth methods"
        );

        stream.write_all(&[0x05, 0x02]).await.unwrap();

        let mut sub = [0u8; 2];
        stream.read_exact(&mut sub).await.unwrap();
        assert_eq!(sub[0], 0x01, "socks5 auth subneg version wrong");
        let ulen = sub[1] as usize;
        let mut uname = vec![0u8; ulen];
        stream.read_exact(&mut uname).await.unwrap();
        let mut plen_buf = [0u8; 1];
        stream.read_exact(&mut plen_buf).await.unwrap();
        let mut pass = vec![0u8; plen_buf[0] as usize];
        stream.read_exact(&mut pass).await.unwrap();
        assert_eq!(uname, b"testuser");
        assert_eq!(pass, b"testpass");

        stream.write_all(&[0x01, 0x00]).await.unwrap();

        let mut conn_hdr = [0u8; 4];
        stream.read_exact(&mut conn_hdr).await.unwrap();
        assert_eq!(conn_hdr[0], 0x05);
        assert_eq!(conn_hdr[1], 0x01, "expected CONNECT cmd");
        assert_eq!(conn_hdr[3], 0x03, "expected domain ATYP");
        let mut dlen = [0u8; 1];
        stream.read_exact(&mut dlen).await.unwrap();
        let mut dom = vec![0u8; dlen[0] as usize];
        stream.read_exact(&mut dom).await.unwrap();
        let mut port = [0u8; 2];
        stream.read_exact(&mut port).await.unwrap();
        assert_eq!(dom, b"target.example.com");
        assert_eq!(u16::from_be_bytes(port), 443);

        stream
            .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await
            .unwrap();
        drop(stream.shutdown().await);
    });

    let proxy_url = format!(
        "socks5://testuser:testpass@{}:{}",
        proxy_addr.ip(),
        proxy_addr.port()
    );
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .proxy(&proxy_url)
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();

    drop(session.get("https://target.example.com/").send().await);

    server.await.unwrap();
    println!("✓ offline SOCKS5 wire bytes OK (RFC 1928 greet + auth + CONNECT)");
}
