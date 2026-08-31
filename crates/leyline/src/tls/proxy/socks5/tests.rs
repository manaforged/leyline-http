use super::*;
use std::time::Duration;
use tokio::net::TcpListener;

async fn pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (client, server) = tokio::join!(TcpStream::connect(addr), listener.accept());
    (client.unwrap(), server.unwrap().0)
}

#[tokio::test]
async fn authenticate_rejects_wrong_subnegotiation_version() {
    let (mut client, mut server) = pair().await;
    let proxy: url::Url = "socks5://user:pass@127.0.0.1:1080".parse().unwrap();
    let auth = auth_request(&proxy).unwrap().unwrap();
    let server_task = tokio::spawn(async move {
        let mut buf = vec![0u8; 64];
        let _ = server.read(&mut buf).await.unwrap();
        server.write_all(&[0x05, 0x00]).await.unwrap();
        server
    });
    let res = authenticate(&mut client, &auth).await;
    assert!(
        res.is_err(),
        "malformed auth VER byte must be rejected, got {res:?}"
    );
    let _ = server_task.await;
}

#[test]
fn auth_request_encodes_rfc_1929_credentials() {
    let proxy: url::Url = "socks5://u%73er:p%40ss@127.0.0.1:1080".parse().unwrap();

    assert_eq!(
        auth_request(&proxy).unwrap(),
        Some(vec![
            0x01, 0x04, b'u', b's', b'e', b'r', 0x04, b'p', b'@', b's', b's'
        ])
    );
}

#[test]
fn auth_request_leaves_no_auth_proxies_unmodified() {
    let proxy: url::Url = "socks5://127.0.0.1:1080".parse().unwrap();

    assert_eq!(auth_request(&proxy).unwrap(), None);
}

#[test]
fn auth_request_accepts_255_byte_credentials() {
    let user = "u".repeat(255);
    let password = "p".repeat(255);
    let proxy: url::Url = format!("socks5://{user}:{password}@127.0.0.1:1080")
        .parse()
        .unwrap();

    let auth = auth_request(&proxy).unwrap().unwrap();
    assert_eq!(auth.len(), 513);
    assert_eq!(&auth[..2], &[0x01, 255]);
    assert_eq!(auth[257], 255);
}

#[test]
fn auth_request_rejects_incomplete_empty_and_oversized_credentials() {
    for proxy in [
        "socks5://user@127.0.0.1:1080",
        "socks5://:pass@127.0.0.1:1080",
        "socks5://user:@127.0.0.1:1080",
        &format!("socks5://{}:pass@127.0.0.1:1080", "u".repeat(256)),
        &format!("socks5://user:{}@127.0.0.1:1080", "p".repeat(256)),
    ] {
        let proxy: url::Url = proxy.parse().unwrap();
        assert!(auth_request(&proxy).is_err(), "{proxy}");
    }
}

#[tokio::test]
async fn send_connect_rejects_hostname_longer_than_255_bytes() {
    let (mut client, _server) = pair().await;
    let long_host = "a".repeat(256);
    let res = tokio::time::timeout(
        Duration::from_secs(1),
        send_connect(&mut client, &long_host, 443),
    )
    .await;
    assert!(
        matches!(res, Ok(Err(_))),
        "256-byte hostname must error immediately, got {res:?}"
    );
}
