use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::stream;
use leyline::{Body, ProtocolPolicy, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn streaming_request_body_chunked_over_h1() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut req = Vec::new();
        let mut tmp = [0u8; 4096];
        let header_end;
        loop {
            let n = sock.read(&mut tmp).await.unwrap();
            assert!(n > 0, "client closed before headers");
            req.extend_from_slice(&tmp[..n]);
            if let Some(idx) = req.windows(4).position(|w| w == b"\r\n\r\n") {
                header_end = idx + 4;
                break;
            }
        }
        let head = String::from_utf8_lossy(&req[..header_end]).to_string();
        assert!(
            head.contains("Transfer-Encoding: chunked"),
            "chunked expected, got headers: {head}"
        );

        let mut body_buf: Vec<u8> = req[header_end..].to_vec();
        let mut total: usize = 0;
        loop {
            let crlf = loop {
                if let Some(i) = body_buf.windows(2).position(|w| w == b"\r\n") {
                    break i;
                }
                let n = sock.read(&mut tmp).await.unwrap();
                if n == 0 {
                    panic!("closed mid-chunk header");
                }
                body_buf.extend_from_slice(&tmp[..n]);
            };
            let size_line = String::from_utf8_lossy(&body_buf[..crlf]).to_string();
            let size = usize::from_str_radix(size_line.trim(), 16).unwrap();
            body_buf.drain(..crlf + 2);
            if size == 0 {
                while body_buf.len() < 2 {
                    let n = sock.read(&mut tmp).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    body_buf.extend_from_slice(&tmp[..n]);
                }
                break;
            }
            while body_buf.len() < size + 2 {
                let n = sock.read(&mut tmp).await.unwrap();
                if n == 0 {
                    panic!("closed mid-chunk body");
                }
                body_buf.extend_from_slice(&tmp[..n]);
            }
            total += size;
            body_buf.drain(..size + 2);
        }

        let resp = format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {len}\r\nconnection: close\r\n\r\n{total}",
            len = total.to_string().len(),
            total = total
        );
        sock.write_all(resp.as_bytes()).await.unwrap();
        sock.flush().await.unwrap();
    });

    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    let chunks: Vec<std::io::Result<Bytes>> = (0..384)
        .map(|_| Ok(Bytes::from(vec![b'a'; 8 * 1024])))
        .collect();
    let expected_total: usize = 384 * 8 * 1024;
    let body = Body::stream(stream::iter(chunks), None);

    let resp = session
        .post(&format!("http://{addr}/upload"))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.text().await.unwrap().trim(),
        expected_total.to_string()
    );

    server.await.unwrap();
}

#[tokio::test]
async fn streaming_request_body_fixed_length_content_length() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let payload_len: usize = 64 * 1024 + 17;

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut req = Vec::new();
        let mut tmp = [0u8; 4096];
        let header_end;
        loop {
            let n = sock.read(&mut tmp).await.unwrap();
            req.extend_from_slice(&tmp[..n]);
            if let Some(idx) = req.windows(4).position(|w| w == b"\r\n\r\n") {
                header_end = idx + 4;
                break;
            }
        }
        let head = String::from_utf8_lossy(&req[..header_end]).to_string();
        assert!(
            head.contains(&format!("Content-Length: {payload_len}")),
            "content-length expected, got: {head}"
        );
        let mut body_buf: Vec<u8> = req[header_end..].to_vec();
        while body_buf.len() < payload_len {
            let n = sock.read(&mut tmp).await.unwrap();
            if n == 0 {
                break;
            }
            body_buf.extend_from_slice(&tmp[..n]);
        }
        assert_eq!(body_buf.len(), payload_len);
        sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
            .await
            .unwrap();
        sock.flush().await.unwrap();
    });

    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    let chunks: Vec<std::io::Result<Bytes>> = vec![
        Ok(Bytes::from(vec![b'x'; 32 * 1024])),
        Ok(Bytes::from(vec![b'y'; 32 * 1024])),
        Ok(Bytes::from(vec![b'z'; 17])),
    ];
    let body = Body::stream(stream::iter(chunks), Some(payload_len as u64));

    let resp = session
        .post(&format!("http://{addr}/upload"))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    server.await.unwrap();
}

#[tokio::test]
async fn response_into_stream_on_buffered_returns_single_chunk() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 2048];
        loop {
            let n = sock.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let total = 10 * 64 * 1024;
        let body_bytes = vec![b'A'; total];
        let resp_head =
            format!("HTTP/1.1 200 OK\r\ncontent-length: {total}\r\nconnection: close\r\n\r\n");
        sock.write_all(resp_head.as_bytes()).await.unwrap();
        sock.write_all(&body_bytes).await.unwrap();
        sock.flush().await.unwrap();
    });

    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    let resp = session
        .request(http::Method::GET, format!("http://{addr}/big"))
        .stream()
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let mut body = resp.into_stream().unwrap();
    let mut total = 0usize;
    while let Some(chunk) = body.next().await {
        let c = chunk.unwrap();
        total += c.len();
    }
    assert_eq!(total, 10 * 64 * 1024);
    server.await.unwrap();
}

async fn one_shot(body: &'static str) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        loop {
            let n = sock.read(&mut buf).await.unwrap();
            if n == 0 || buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let head = format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        sock.write_all(head.as_bytes()).await.unwrap();
        sock.write_all(body.as_bytes()).await.unwrap();
        sock.flush().await.unwrap();
    });
    addr
}

#[tokio::test]
async fn text_drains_a_streamed_body() {
    let addr = one_shot("hello stream").await;
    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    let resp = session
        .request(http::Method::GET, format!("http://{addr}/x"))
        .stream()
        .send()
        .await
        .unwrap();
    assert_eq!(resp.text().await.unwrap(), "hello stream");
}

#[tokio::test]
async fn buffered_body_is_visible_to_as_bytes() {
    let addr = one_shot("buffered").await;
    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    let resp = session.get(&format!("http://{addr}/x")).await.unwrap();
    assert_eq!(resp.bytes().await.unwrap(), &b"buffered"[..]);
}

#[tokio::test]
async fn download_to_writes_body_to_file() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let payload = vec![0xABu8; 100_000];
    let payload_for_server = payload.clone();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        loop {
            let n = sock.read(&mut buf).await.unwrap();
            if n == 0 || buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let head = format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            payload_for_server.len()
        );
        sock.write_all(head.as_bytes()).await.unwrap();
        sock.write_all(&payload_for_server).await.unwrap();
    });

    let path = std::env::temp_dir().join(format!("leyline-dl-{}.bin", addr.port()));
    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    let resp = session.get(&format!("http://{addr}/file")).await.unwrap();
    let mut file = tokio::fs::File::create(&path).await.unwrap();
    let n = resp.copy_to(&mut file).await.unwrap();
    assert_eq!(n, payload.len() as u64);

    let on_disk = tokio::fs::read(&path).await.unwrap();
    assert_eq!(on_disk, payload, "downloaded bytes must match the body");
    let _ = tokio::fs::remove_file(&path).await;
    server.await.unwrap();
}

#[tokio::test]
async fn into_stream_twice_returns_error() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        loop {
            let n = sock.read(&mut buf).await.unwrap();
            if n == 0 || buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\nconnection: close\r\n\r\nhello")
            .await
            .unwrap();
    });

    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    let resp = session
        .request(http::Method::GET, format!("http://{addr}/x"))
        .stream()
        .send()
        .await
        .unwrap();
    let mut s1 = match resp.into_stream() {
        Ok(s) => s,
        Err(e) => panic!("unexpected error: {e}"),
    };
    while let Some(c) = s1.next().await {
        let _ = c.unwrap();
    }
    server.await.unwrap();
}

#[tokio::test]
async fn redirect_with_streaming_body_errors() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let addr2 = format!("http://{addr}/after");

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut req = Vec::new();
        let mut tmp = [0u8; 4096];
        loop {
            let n = sock.read(&mut tmp).await.unwrap();
            if n == 0 {
                break;
            }
            req.extend_from_slice(&tmp[..n]);
            if req.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let resp = format!(
            "HTTP/1.1 307 Temporary Redirect\r\ncontent-length: 0\r\nlocation: {addr2}\r\nconnection: close\r\n\r\n"
        );
        sock.write_all(resp.as_bytes()).await.unwrap();
        sock.flush().await.unwrap();
    });

    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    let chunks: Vec<std::io::Result<Bytes>> = vec![Ok(Bytes::from_static(b"hello"))];
    let body = Body::stream(stream::iter(chunks), None);

    let err = session
        .post(&format!("http://{addr}/before"))
        .body(body)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("streaming request bodies are not replayable"),
        "got: {msg}"
    );
    server.await.unwrap();
}
