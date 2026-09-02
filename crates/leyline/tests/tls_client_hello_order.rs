//! Wire-level ClientHello ordering regressions anchored to real browser captures.
#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use leyline::Browser;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

const FIREFOX_152_CIPHERS: &[u16] = &[
    0x1301, 0x1303, 0x1302, 0xc02b, 0xc02f, 0xcca9, 0xcca8, 0xc02c, 0xc030, 0xc00a, 0xc013, 0xc014,
    0x009c, 0x009d, 0x002f, 0x0035,
];
const FIREFOX_152_EXTENSIONS: &[u16] = &[
    0, 23, 65281, 10, 11, 35, 16, 5, 34, 18, 51, 43, 13, 45, 28, 27, 65037,
];

/// Captured from official Firefox 152.0.6 on macOS on 2026-07-16.
#[tokio::test]
async fn firefox_152_matches_captured_cipher_and_extension_order() {
    let (ciphers, extensions) = capture_client_hello(Browser::Firefox152).await;
    assert_eq!(ciphers, FIREFOX_152_CIPHERS);
    assert_eq!(extensions, FIREFOX_152_EXTENSIONS);
    assert_eq!(
        declared_extension_order(Browser::Firefox152),
        FIREFOX_152_EXTENSIONS,
        "firefox/152.toml drifted from the captured extension order"
    );
}

/// The declared order is applied per profile, not baked into the TLS builder: Firefox 150 must put its own `extension_permutation` on the wire too.
#[tokio::test]
async fn firefox_150_client_hello_follows_its_declared_extension_order() {
    let (_, extensions) = capture_client_hello(Browser::Firefox150).await;
    assert_eq!(extensions, declared_extension_order(Browser::Firefox150));
}

/// Chrome 147+ advertises Trust Anchor Identifiers (0xCA34) with an empty list.
#[tokio::test]
async fn chrome_147_client_hello_carries_trust_anchor_identifiers() {
    let (_, extensions) = capture_client_hello(Browser::Chrome147).await;
    assert!(
        extensions.contains(&0xca34),
        "trust_anchors extension absent; JA4 drops to t13d1516: {extensions:04x?}"
    );
}

/// The fixed ClientHello extension order a built-in profile declares.
fn declared_extension_order(browser: Browser) -> Vec<u16> {
    leyline::profile::ProfileRegistry::builtin()
        .get_browser(browser)
        .expect("built-in profile")
        .tls
        .extension_permutation
        .clone()
        .expect("profile declares a fixed extension order")
}

/// Drive one real handshake attempt at a local listener that never answers, and return the `(cipher, extension)` IDs of the ClientHello it produced.
async fn capture_client_hello(browser: Browser) -> (Vec<u16>, Vec<u16>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_tls_record(&mut stream).await
    });

    let session = leyline::Session::builder()
        .browser(browser)
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap();
    let _ = session
        .get(&format!("https://localhost:{}/", addr.port()))
        .await;

    let record = server.await.unwrap();
    parse_client_hello(&record).expect("valid ClientHello")
}

async fn read_tls_record(stream: &mut tokio::net::TcpStream) -> Vec<u8> {
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
