#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use leyline::http::header::HOST;
use leyline::{ProtocolPolicy, ProxyConfig, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

type Heads = Arc<Mutex<Vec<String>>>;

async fn serve(reply: String, heads: Heads) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut head = Vec::new();
            let mut buf = [0u8; 1024];
            while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                match socket.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(read) => head.extend_from_slice(&buf[..read]),
                }
            }
            heads
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&head).into_owned());
            drop(socket.write_all(reply.as_bytes()).await);
        }
    });
    addr
}

fn host(head: &str) -> Option<&str> {
    head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case(HOST.as_str())
            .then(|| value.trim())
    })
}

#[tokio::test]
async fn a_caller_host_header_is_dropped_on_a_cross_origin_redirect() {
    let landed = Heads::default();
    let target = serve(
        "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".into(),
        Arc::clone(&landed),
    )
    .await;
    let origin = serve(
        format!(
            "HTTP/1.1 302 Found\r\nLocation: http://{target}/next\r\nContent-Length: 0\r\n\r\n"
        ),
        Heads::default(),
    )
    .await;
    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .proxy(ProxyConfig::new().env(false))
        .build()
        .unwrap();
    session
        .get(format!("http://{origin}/"))
        .header(HOST, "caller.example")
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let heads = landed.lock().unwrap();
    assert_eq!(heads.len(), 1);
    assert_eq!(host(&heads[0]), Some(target.to_string().as_str()));
}
