//! Integration tests for `multipart/form-data` bodies.

use std::io::Write;

use leyline::core::Session;
use leyline::core::multipart::{Form, Part};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Read the full request (headers + body) off the socket. The tests
/// here set `Content-Length` so we can read exactly.
async fn read_full_request(sock: &mut tokio::net::TcpStream) -> (String, Vec<u8>) {
    let mut buf = [0u8; 8192];
    let mut acc = Vec::new();
    // Read headers first.
    let header_end;
    loop {
        let n = sock.read(&mut buf).await.unwrap();
        assert!(n > 0, "client closed");
        acc.extend_from_slice(&buf[..n]);
        if let Some(idx) = acc.windows(4).position(|w| w == b"\r\n\r\n") {
            header_end = idx + 4;
            break;
        }
    }
    let head_text = String::from_utf8_lossy(&acc[..header_end]).to_string();
    // Parse Content-Length.
    let mut cl: Option<usize> = None;
    for line in head_text.split("\r\n") {
        if let Some(v) = line.strip_prefix("Content-Length: ") {
            cl = v.parse().ok();
        } else if let Some(v) = line.strip_prefix("content-length: ") {
            cl = v.parse().ok();
        }
    }
    let content_len = cl.expect("content-length present");
    let mut body = acc[header_end..].to_vec();
    while body.len() < content_len {
        let n = sock.read(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
    }
    body.truncate(content_len);
    (head_text, body)
}

#[tokio::test]
async fn text_plus_text_form_roundtrips() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let (head, body) = read_full_request(&mut sock).await;
        // Extract boundary from content-type.
        let ct = head
            .split("\r\n")
            .find(|l| l.to_lowercase().starts_with("content-type:"))
            .unwrap();
        assert!(ct.contains("multipart/form-data"), "{ct}");
        let boundary = ct.split("boundary=").nth(1).unwrap().trim().to_string();

        let body_text = String::from_utf8_lossy(&body).to_string();
        // Three distinct occurrences: 2 part openers + 1 closer.
        let opener = format!("--{boundary}\r\n");
        let closer = format!("--{boundary}--\r\n");
        assert_eq!(body_text.matches(&opener).count(), 2, "body: {body_text}");
        assert!(body_text.ends_with(&closer), "body: {body_text}");
        assert!(body_text.contains("name=\"user\""), "body: {body_text}");
        assert!(body_text.contains("name=\"role\""), "body: {body_text}");
        assert!(body_text.contains("alice"), "body: {body_text}");
        assert!(body_text.contains("admin"), "body: {body_text}");

        sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
            .await
            .unwrap();
        sock.flush().await.unwrap();
    });

    let session = Session::builder().http1().build().unwrap();
    let form = Form::new().text("user", "alice").text("role", "admin");
    let resp = session
        .post(&format!("http://{addr}/submit"))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    server.await.unwrap();
}

#[tokio::test]
async fn text_plus_bytes_part() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let (_head, body) = read_full_request(&mut sock).await;
        let body_text = String::from_utf8_lossy(&body).to_string();
        assert!(body_text.contains("name=\"caption\""), "{body_text}");
        assert!(body_text.contains("name=\"image\""), "{body_text}");
        assert!(body_text.contains("filename=\"pixel.png\""), "{body_text}");
        assert!(body_text.contains("Content-Type: image/png"), "{body_text}");
        assert!(body_text.contains("hello-world-bytes"), "{body_text}");
        sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
            .await
            .unwrap();
        sock.flush().await.unwrap();
    });

    let session = Session::builder().http1().build().unwrap();
    let form = Form::new().text("caption", "a tiny image").part(
        "image",
        Part::bytes(bytes::Bytes::from_static(b"hello-world-bytes"))
            .filename("pixel.png")
            .mime("image/png"),
    );
    let resp = session
        .post(&format!("http://{addr}/upload"))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    server.await.unwrap();
}

#[tokio::test]
async fn text_plus_file() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Write a temp file with known contents.
    let mut tf = tempfile_like("leyline-mp-test.bin");
    tf.write_all(b"file-bytes-under-test-0123456789").unwrap();
    tf.flush().unwrap();
    let path = tf.path.clone();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let (_head, body) = read_full_request(&mut sock).await;
        let body_text = String::from_utf8_lossy(&body).to_string();
        assert!(
            body_text.contains("file-bytes-under-test-0123456789"),
            "{body_text}"
        );
        assert!(body_text.contains("leyline-mp-test.bin"), "{body_text}");
        assert!(body_text.contains("filename=\""), "{body_text}");
        sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
            .await
            .unwrap();
        sock.flush().await.unwrap();
    });

    let session = Session::builder().http1().build().unwrap();
    let form = Form::new()
        .text("user", "alice")
        .file("payload", &path)
        .unwrap();
    let resp = session
        .post(&format!("http://{addr}/upload"))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    server.await.unwrap();
    drop(tf);
}

#[test]
fn boundaries_differ_between_forms() {
    let a = Form::new();
    let b = Form::new();
    assert_ne!(a.boundary(), b.boundary());
}

struct TempFile {
    path: std::path::PathBuf,
    file: std::fs::File,
}

impl TempFile {
    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.file.write_all(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Create a deterministic temp file path under the system temp dir
/// (avoids pulling in the `tempfile` crate for a single call site).
fn tempfile_like(name: &str) -> TempFile {
    let dir = std::env::temp_dir();
    let unique = format!(
        "{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        name
    );
    let path = dir.join(unique);
    let file = std::fs::File::create(&path).unwrap();
    TempFile { path, file }
}

// ─────────────────────────────────────────────────────────────────────
// Regression gates: header-injection defence. Prior to the fix
// a `name` or `filename` containing CR/LF would escape the
// Content-Disposition header and let an attacker forge headers or
// break out of the multipart body entirely (CWE-93).
// ─────────────────────────────────────────────────────────────────────

use futures_util::StreamExt;

/// Directly pull the first chunk out of a Form to inspect its
/// `Content-Disposition` serialization without going over the wire.
async fn first_part_header(form: Form) -> std::io::Result<bytes::Bytes> {
    let body = form.into_stream_body_for_test();
    let mut stream = match body {
        leyline::core::Body::Stream { stream, .. } => stream,
        _ => panic!("multipart body must be Stream"),
    };
    match stream.next().await {
        Some(Ok(b)) => Ok(b),
        Some(Err(e)) => Err(e),
        None => Err(std::io::Error::other("empty form")),
    }
}

#[tokio::test]
async fn filename_with_crlf_is_rejected() {
    let form = Form::new().part(
        "upload",
        Part::bytes(b"nope".to_vec()).filename("x\"\r\nInjected: yes\r\n\r\n--evil\r\n"),
    );
    let err = first_part_header(form).await.expect_err("must reject");
    let msg = format!("{err}");
    assert!(
        msg.contains("filename") && msg.contains("control"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn name_with_crlf_is_rejected() {
    let form = Form::new().text("field\r\nInjected: yes", "v");
    let err = first_part_header(form).await.expect_err("must reject");
    let msg = format!("{err}");
    assert!(
        msg.contains("name") && msg.contains("control"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn filename_with_dquote_is_backslash_escaped() {
    let form = Form::new().part(
        "upload",
        Part::bytes(b"body".to_vec()).filename("evil\"file.txt"),
    );
    let chunk = first_part_header(form).await.expect("accept with escape");
    let s = std::str::from_utf8(&chunk).expect("utf8");
    assert!(s.contains(r#"filename="evil\"file.txt""#), "chunk: {s}");
}
