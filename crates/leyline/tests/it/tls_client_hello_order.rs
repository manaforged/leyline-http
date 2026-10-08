#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use leyline::Browser;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const FIREFOX_152_CIPHERS: &[u16] = &[
    0x1301, 0x1303, 0x1302, 0xc02b, 0xc02f, 0xcca9, 0xcca8, 0xc02c, 0xc030, 0xc00a, 0xc013, 0xc014,
    0x009c, 0x009d, 0x002f, 0x0035,
];
const FIREFOX_152_EXTENSIONS: &[u16] = &[
    0, 23, 65281, 10, 11, 35, 16, 5, 34, 18, 51, 43, 13, 45, 28, 27, 65037,
];

#[tokio::test]
async fn firefox_152_matches_captured_cipher_and_extension_order() {
    let (ciphers, extensions) = capture_client_hello(Browser::Firefox152, Route::Direct).await;
    assert_eq!(ciphers, FIREFOX_152_CIPHERS);
    assert_eq!(extensions, FIREFOX_152_EXTENSIONS);
    assert_eq!(
        declared_extension_order(Browser::Firefox152),
        FIREFOX_152_EXTENSIONS,
        "firefox/152.toml drifted from the captured extension order"
    );
}

#[tokio::test]
async fn firefox_150_client_hello_follows_its_declared_extension_order() {
    let (_, extensions) = capture_client_hello(Browser::Firefox150, Route::Direct).await;
    assert_eq!(extensions, declared_extension_order(Browser::Firefox150));
}

#[tokio::test]
async fn chrome_154_client_hello_carries_trust_anchor_identifiers() {
    let (_, extensions) = capture_client_hello(Browser::Chrome154, Route::Direct).await;
    assert!(
        extensions.contains(&0xca34),
        "trust_anchors extension absent; JA4 drops to t13d1516: {extensions:04x?}"
    );
}

#[tokio::test]
async fn an_http_connect_tunnel_leaves_the_client_hello_unchanged() {
    assert_eq!(
        capture_client_hello(Browser::Firefox152, Route::HttpConnect).await,
        capture_client_hello(Browser::Firefox152, Route::Direct).await
    );
}

#[cfg(feature = "socks")]
#[tokio::test]
async fn a_socks5_tunnel_leaves_the_client_hello_unchanged() {
    assert_eq!(
        capture_client_hello(Browser::Firefox152, Route::Socks5).await,
        capture_client_hello(Browser::Firefox152, Route::Direct).await
    );
}

#[derive(Clone, Copy)]
enum Route {
    Direct,
    HttpConnect,
    #[cfg(feature = "socks")]
    Socks5,
}

impl Route {
    fn proxy(self, port: u16) -> Option<String> {
        match self {
            Route::Direct => None,
            Route::HttpConnect => Some(format!("http://127.0.0.1:{port}")),
            #[cfg(feature = "socks")]
            Route::Socks5 => Some(format!("socks5://127.0.0.1:{port}")),
        }
    }

    async fn accept(self, stream: &mut TcpStream) {
        match self {
            Route::Direct => {}
            Route::HttpConnect => {
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).await.unwrap();
                    head.push(byte[0]);
                }
                assert!(
                    head.starts_with(b"CONNECT origin.test:443 HTTP/1.1\r\n"),
                    "{}",
                    String::from_utf8_lossy(&head)
                );
                stream
                    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                    .await
                    .unwrap();
            }
            #[cfg(feature = "socks")]
            Route::Socks5 => {
                let mut greeting = [0u8; 2];
                stream.read_exact(&mut greeting).await.unwrap();
                let mut methods = vec![0u8; greeting[1] as usize];
                stream.read_exact(&mut methods).await.unwrap();
                stream.write_all(&[0x05, 0x00]).await.unwrap();
                let mut request = [0u8; 5];
                stream.read_exact(&mut request).await.unwrap();
                assert_eq!(request[..4], [0x05, 0x01, 0x00, 0x03]);
                let mut target = vec![0u8; request[4] as usize + 2];
                stream.read_exact(&mut target).await.unwrap();
                assert_eq!(&target[..target.len() - 2], b"origin.test");
                stream
                    .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                    .await
                    .unwrap();
            }
        }
    }
}

fn declared_extension_order(browser: Browser) -> Vec<u16> {
    browser
        .profile()
        .tls
        .extension_permutation
        .clone()
        .expect("profile declares a fixed extension order")
}

async fn capture_client_hello(browser: Browser, route: Route) -> (Vec<u16>, Vec<u16>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        route.accept(&mut stream).await;
        read_tls_record(&mut stream).await
    });

    let mut builder = leyline::Session::builder()
        .browser(browser)
        .timeout(std::time::Duration::from_secs(3));
    let url = match route.proxy(port) {
        Some(proxy) => {
            builder = builder.proxy(proxy);
            "https://origin.test/".to_owned()
        }
        None => format!("https://localhost:{port}/"),
    };
    drop(builder.build().unwrap().get(url).await);

    let record = server.await.unwrap();
    parse_client_hello(&record).expect("valid ClientHello")
}

async fn read_tls_record(stream: &mut TcpStream) -> Vec<u8> {
    let mut record = Vec::with_capacity(4096);
    let mut buf = [0u8; 2048];
    loop {
        match tokio::time::timeout(std::time::Duration::from_millis(750), stream.read(&mut buf))
            .await
        {
            Ok(Ok(0)) | Err(_) => break,
            Ok(Ok(n)) => {
                record.extend_from_slice(&buf[..n]);
                if record.len() >= 5 {
                    let len = u16::from_be_bytes([record[3], record[4]]) as usize;
                    if record.len() >= 5 + len {
                        break;
                    }
                }
            }
            Ok(Err(_)) => break,
        }
    }
    record
}

fn parse_client_hello(record: &[u8]) -> Option<(Vec<u16>, Vec<u16>)> {
    if record.len() < 5 || record[0] != 0x16 {
        return None;
    }
    let record_len = u16::from_be_bytes([record[3], record[4]]) as usize;
    let handshake = record.get(5..5 + record_len)?;
    if handshake.first().copied()? != 0x01 || handshake.len() < 4 {
        return None;
    }
    let hello_len =
        ((handshake[1] as usize) << 16) | ((handshake[2] as usize) << 8) | handshake[3] as usize;
    let hello = handshake.get(4..4 + hello_len)?;

    let mut offset = 34;
    offset += 1 + *hello.get(offset)? as usize;

    let cipher_len = read_u16(hello, offset)? as usize;
    offset += 2;
    let cipher_bytes = hello.get(offset..offset + cipher_len)?;
    let ciphers = cipher_bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|bytes| u16::from_be_bytes(*bytes))
        .collect();
    offset += cipher_len;

    offset += 1 + *hello.get(offset)? as usize;
    let extensions_len = read_u16(hello, offset)? as usize;
    offset += 2;
    let extensions_end = offset.checked_add(extensions_len)?;
    let mut extensions = Vec::new();
    while offset < extensions_end {
        let extension = read_u16(hello, offset)?;
        let len = read_u16(hello, offset + 2)? as usize;
        extensions.push(extension);
        offset = offset.checked_add(4 + len)?;
    }
    (offset == extensions_end).then_some((ciphers, extensions))
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes([
        *bytes.get(offset)?,
        *bytes.get(offset + 1)?,
    ]))
}
