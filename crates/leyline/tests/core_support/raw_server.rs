#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
#![allow(dead_code)]
use std::collections::VecDeque;
use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Clone)]
pub struct CapturedRequest {
    pub raw: Vec<u8>,
    pub request_line: String,
    pub headers: Vec<(String, String)>,
}

impl CapturedRequest {
    pub fn header_values(&self, name: &str) -> Vec<String> {
        self.headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
            .collect()
    }

    pub fn header_count(&self, name: &str) -> usize {
        self.headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case(name))
            .count()
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.raw).to_string()
    }

    fn from_raw(raw: Vec<u8>) -> Self {
        let text = String::from_utf8_lossy(&raw);
        let mut lines = text.split("\r\n");
        let request_line = lines.next().unwrap_or_default().to_string();
        let mut headers = Vec::new();
        for line in lines {
            if line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                headers.push((name.to_string(), value.trim_start().to_string()));
            }
        }

        Self {
            raw,
            request_line,
            headers,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RawResponse {
    status: u16,
    reason: &'static str,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl RawResponse {
    pub fn ok() -> Self {
        Self::new(200, "OK", b"ok".to_vec())
    }

    pub fn redirect(location: impl Into<String>) -> Self {
        Self::new(302, "Found", Vec::new()).header("location", location.into())
    }

    pub fn status(status: u16, reason: &'static str) -> Self {
        Self::new(status, reason, Vec::new())
    }

    fn new(status: u16, reason: &'static str, body: Vec<u8>) -> Self {
        Self {
            status,
            reason,
            headers: Vec::new(),
            body,
        }
    }

    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(format!("HTTP/1.1 {} {}\r\n", self.status, self.reason).as_bytes());
        out.extend_from_slice(format!("content-length: {}\r\n", self.body.len()).as_bytes());
        out.extend_from_slice(b"connection: close\r\n");
        for (name, value) in &self.headers {
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(b": ");
            out.extend_from_slice(value.as_bytes());
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(&self.body);
        out
    }
}

pub struct RawServer {
    addr: SocketAddr,
    requests: tokio::sync::mpsc::Receiver<CapturedRequest>,
    task: tokio::task::JoinHandle<()>,
}

impl RawServer {
    pub async fn start(responses: Vec<RawResponse>) -> Self {
        Self::serve(Self::bind().await, responses)
    }

    pub async fn bind() -> tokio::net::TcpListener {
        tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap()
    }

    pub fn serve(listener: tokio::net::TcpListener, responses: Vec<RawResponse>) -> Self {
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(responses.len().max(1));
        let task = tokio::spawn(async move {
            let mut responses = VecDeque::from(responses);
            while let Some(response) = responses.pop_front() {
                let (mut socket, _) = listener.accept().await.unwrap();
                let raw = read_request_head(&mut socket).await.unwrap();
                tx.send(CapturedRequest::from_raw(raw)).await.unwrap();
                socket.write_all(&response.to_bytes()).await.unwrap();
                socket.flush().await.unwrap();
            }
        });

        Self {
            addr,
            requests: rx,
            task,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    pub async fn next_request(&mut self) -> CapturedRequest {
        self.requests
            .recv()
            .await
            .expect("raw server closed before capturing request")
    }

    pub async fn finish(self) {
        self.task.await.unwrap();
    }
}

async fn read_request_head(socket: &mut tokio::net::TcpStream) -> std::io::Result<Vec<u8>> {
    let mut raw = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        let n = socket.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        raw.extend_from_slice(&buf[..n]);
        if raw.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    Ok(raw)
}
