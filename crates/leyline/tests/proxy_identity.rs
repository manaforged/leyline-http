use leyline::testing::{TestResponse, TestServer, queue};
use leyline::{Browser, Family, Identity, Kind, Platform, ProxyPool, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn proxy_url(server: &TestServer) -> String {
    format!("http://{}", server.addr())
}

#[tokio::test]
async fn proxies_that_share_an_identity_keep_separate_jars() {
    let first = TestServer::http(queue([TestResponse::new(200)
        .close()
        .header("set-cookie", "sid=1; Path=/")]))
    .await
    .unwrap();
    let second = TestServer::http(queue([TestResponse::new(200).close()]))
        .await
        .unwrap();
    let id = Identity::locked(Browser::latest(Family::Firefox), Platform::Windows);
    let session = Session::builder()
        .browser(Browser::latest(Family::Chrome))
        .proxy_pool(ProxyPool::identified([
            (proxy_url(&first), id),
            (proxy_url(&second), id),
        ]))
        .build()
        .unwrap();
    session.get("http://origin.test/a").await.unwrap();
    session.get("http://origin.test/b").await.unwrap();
    let sent = second.next_request().await.unwrap();
    assert_eq!(sent.target, "http://origin.test/b");
    assert!(
        sent.header_values("cookie").is_empty(),
        "{:?}",
        sent.header_values("cookie")
    );
}

#[test]
fn an_identified_pool_needs_a_browser_session() {
    let id = Identity::locked(Browser::latest(Family::Firefox), Platform::Windows);
    let err = Session::builder()
        .proxy_pool(ProxyPool::identified([("http://127.0.0.1:9", id)]))
        .build()
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Config);
}

async fn connect_proxy() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut inbound, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if inbound.read(&mut byte).await.unwrap_or(0) == 0 {
                        return;
                    }
                    head.push(byte[0]);
                }
                let text = String::from_utf8_lossy(&head);
                let target = text
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_owned();
                let mut outbound = TcpStream::connect(target).await.unwrap();
                inbound
                    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                    .await
                    .unwrap();
                drop(tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await);
            });
        }
    });
    port
}

#[tokio::test]
async fn saved_state_never_holds_the_proxy_password() {
    let server = TestServer::https(queue([TestResponse::new(200), TestResponse::new(200)]))
        .await
        .unwrap();
    let port = connect_proxy().await;
    let session = Session::builder()
        .browser(Browser::default())
        .proxy(format!("http://alice:hunter2secret@127.0.0.1:{port}"))
        .tls_trust(server.trust())
        .build()
        .unwrap();
    session
        .get(server.url("/"))
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    session.get(server.url("/again")).await.unwrap();
    let saved = serde_json::to_string(&session.state()).unwrap();
    assert!(
        saved.contains("127.0.0.1"),
        "no TLS session was saved: {saved}"
    );
    assert!(!saved.contains("hunter2secret"), "{saved}");
    assert!(!saved.contains("alice"), "{saved}");
    assert!(saved.contains("#proxy-credentials="), "{saved}");
}
