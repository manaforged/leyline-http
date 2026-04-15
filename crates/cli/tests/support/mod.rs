//! Minimal local HTTP/1.1 server for offline integration tests.
//!
//! Spawns a `std::net::TcpListener` on 127.0.0.1 and a background
//! thread that accepts **one** connection, runs the given handler
//! once, then closes. Handlers get the raw request bytes and return
//! the raw response bytes — no HTTP parsing helpers, because the
//! tests want byte-level control over the wire.
//!
//! The CLI crate uses this instead of `hyper` so the test
//! dependency tree stays small.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread::JoinHandle;

/// Background server handle. Dropping it joins the thread so a test
/// panic doesn't leak the listener.
pub struct LocalServer {
    pub url: String,
    handle: Option<JoinHandle<()>>,
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Start a one-shot HTTP server that runs `handler` against the raw
/// request bytes and replies with whatever the handler returns.
pub fn start<F>(handler: F) -> LocalServer
where
    F: FnOnce(&[u8]) -> Vec<u8> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}/");
    let handler = Arc::new(std::sync::Mutex::new(Some(handler)));

    let handle = std::thread::spawn(move || {
        listener
            .set_nonblocking(false)
            .expect("blocking accept mode");
        if let Ok((mut sock, _)) = listener.accept() {
            let mut buf = Vec::with_capacity(4096);
            let mut chunk = [0u8; 4096];
            // Read until the header terminator. Then try to read
            // Content-Length bytes of body, if declared.
            loop {
                let n = sock.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if let Some(header_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let body_start = header_end + 4;
                    let content_length = parse_content_length(&buf[..header_end]);
                    let have_body = buf.len() - body_start;
                    if have_body >= content_length {
                        break;
                    }
                }
                if buf.len() > 8 * 1024 * 1024 {
                    break; // safety cap
                }
            }
            let resp = if let Some(h) = handler.lock().unwrap().take() {
                h(&buf)
            } else {
                canned_200_text("")
            };
            let _ = sock.write_all(&resp);
            let _ = sock.shutdown(std::net::Shutdown::Write);
        }
    });

    LocalServer {
        url,
        handle: Some(handle),
    }
}

/// Build a simple `200 OK` response with a plaintext body.
pub fn canned_200_text(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes()
}

/// Build a `404 Not Found` response (for exit-code tests).
pub fn canned_404() -> Vec<u8> {
    let body = "nope";
    format!(
        "HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes()
}

/// Build a `503 Service Unavailable` response.
pub fn canned_503() -> Vec<u8> {
    let body = "down";
    format!(
        "HTTP/1.1 503 Service Unavailable\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes()
}

/// Extract the body portion of a raw HTTP request.
pub fn body_of(req: &[u8]) -> &[u8] {
    if let Some(header_end) = req.windows(4).position(|w| w == b"\r\n\r\n") {
        &req[header_end + 4..]
    } else {
        &[]
    }
}

fn parse_content_length(head: &[u8]) -> usize {
    let text = std::str::from_utf8(head).unwrap_or("");
    for line in text.split("\r\n") {
        if let Some(rest) = line
            .split_once(':')
            .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
            .map(|(_, v)| v.trim())
        {
            return rest.parse().unwrap_or(0);
        }
    }
    0
}
