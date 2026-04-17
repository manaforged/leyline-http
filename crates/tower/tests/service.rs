//! End-to-end tower Service tests for the Leyline adapter.

use leyline::{Request, Session};
use leyline_tower::LeylineService;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::{Service, ServiceExt};

#[tokio::test]
async fn ready_then_call_round_trips() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 2048];
        loop {
            let n = sock.read(&mut buf).await.unwrap();
            if n == 0 || buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        sock.write_all(
            b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok",
        )
        .await
        .unwrap();
        sock.flush().await.unwrap();
    });

    let session = Session::builder().http1().build().unwrap();
    let mut svc = LeylineService::new(session);

    let req = Request::get(format!("http://{addr}/hello"));
    let resp = svc.ready().await.unwrap().call(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text(), "ok");
    server.await.unwrap();
}

#[test]
fn service_clone_is_cheap_and_shares_session() {
    let session = Session::builder().http1().build().unwrap();
    let a = LeylineService::new(session);
    let b = a.clone();
    // Both hold references to the same session arc; compile-check only.
    let _ = (a, b);
}
