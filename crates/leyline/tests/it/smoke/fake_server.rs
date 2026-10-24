use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Clone, Copy)]
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) headers: &'static str,
    pub(super) body: &'static str,
}

pub(super) async fn spawn_replies(replies: Vec<Reply>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let fallback = *replies.last().expect("at least one reply");
    let queue = Arc::new(Mutex::new(VecDeque::from(replies)));
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let queue = Arc::clone(&queue);
            tokio::spawn(async move {
                let _ = read_headers(&mut socket).await;
                let reply = {
                    let mut guard = queue.lock().unwrap_or_else(|err| err.into_inner());
                    guard.pop_front().unwrap_or(fallback)
                };
                let raw = format!(
                    "HTTP/1.1 {status} REPLY\r\nContent-Length: {len}\r\nConnection: close\r\n{headers}\r\n{body}",
                    status = reply.status,
                    len = reply.body.len(),
                    headers = reply.headers,
                    body = reply.body,
                );
                drop(socket.write_all(raw.as_bytes()).await);
            });
        }
    });
    format!("http://{addr}/get")
}

pub(super) async fn spawn_hung() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let _ = read_headers(&mut socket).await;
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            });
        }
    });
    format!("http://{addr}/get")
}

async fn read_headers(socket: &mut TcpStream) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    while buf.len() < 16 * 1024 {
        let Ok(n) = socket.read(&mut tmp).await else {
            return;
        };
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|window| window == b"\r\n\r\n") {
            return;
        }
    }
}
