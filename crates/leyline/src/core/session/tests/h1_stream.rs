use super::super::Session;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn read_request_head(sock: &mut tokio::net::TcpStream) -> bool {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        let n = sock.read(&mut tmp).await.unwrap();
        if n == 0 {
            return !buf.is_empty();
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            return true;
        }
    }
}

#[tokio::test]
async fn streamed_chunked_response_reassembles() {
    use futures_util::StreamExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        read_request_head(&mut sock).await;
        sock.write_all(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
              7\r\nHello, \r\nA\r\nstreaming \r\n6\r\nworld!\r\n0\r\n\r\n",
        )
        .await
        .unwrap();
    });

    let resp = Session::new()
        .request(http::Method::GET, format!("http://{addr}/x"))
        .stream()
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let mut body = Vec::new();
    let mut stream = resp.into_stream().unwrap();
    while let Some(chunk) = stream.next().await {
        body.extend_from_slice(&chunk.unwrap());
    }
    assert_eq!(body, b"Hello, streaming world!");
    server.await.unwrap();
}

#[tokio::test]
async fn streamed_fixed_length_response_reassembles() {
    use futures_util::StreamExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let payload = b"the quick brown fox ".repeat(500);
    let payload_srv = payload.clone();
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        read_request_head(&mut sock).await;
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            payload_srv.len()
        );
        sock.write_all(head.as_bytes()).await.unwrap();
        sock.write_all(&payload_srv).await.unwrap();
    });

    let resp = Session::new()
        .request(http::Method::GET, format!("http://{addr}/x"))
        .stream()
        .send()
        .await
        .unwrap();
    let mut body = Vec::new();
    let mut stream = resp.into_stream().unwrap();
    while let Some(chunk) = stream.next().await {
        body.extend_from_slice(&chunk.unwrap());
    }
    assert_eq!(body, payload);
    server.await.unwrap();
}

#[tokio::test]
async fn streamed_connection_is_reused_after_full_drain() {
    use futures_util::StreamExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        for body in [b"first".as_slice(), b"second".as_slice()] {
            assert!(read_request_head(&mut sock).await, "expected a request");
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                body.len()
            );
            sock.write_all(head.as_bytes()).await.unwrap();
            sock.write_all(body).await.unwrap();
        }
    });

    let session = Session::new();
    let r1 = session
        .request(http::Method::GET, format!("http://{addr}/a"))
        .stream()
        .send()
        .await
        .unwrap();
    let mut b1 = Vec::new();
    let mut s1 = r1.into_stream().unwrap();
    while let Some(chunk) = s1.next().await {
        b1.extend_from_slice(&chunk.unwrap());
    }
    assert_eq!(b1, b"first");

    let r2 = session.get(format!("http://{addr}/b")).await.unwrap();
    assert_eq!(r2.text().await.unwrap(), "second");
    server.await.unwrap();
}

#[tokio::test]
async fn streamed_read_timeout_fires_on_stall() {
    use futures_util::StreamExt;
    use std::time::Duration;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        read_request_head(&mut sock).await;
        sock.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
    });

    let session = Session::builder()
        .timeout(crate::TimeoutConfig::new().read(Duration::from_millis(200)))
        .build()
        .unwrap();
    let resp = session
        .request(http::Method::GET, format!("http://{addr}/x"))
        .stream()
        .send()
        .await
        .unwrap();
    let mut stream = resp.into_stream().unwrap();
    assert_eq!(&stream.next().await.unwrap().unwrap()[..], b"hello");
    let err = stream.next().await.unwrap().unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::TimedOut, "{err}");
    server.abort();
}

#[tokio::test]
async fn streamed_connection_dropped_when_consumer_drops_early() {
    use futures_util::StreamExt;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accepts = Arc::new(AtomicUsize::new(0));
    let accepts_srv = accepts.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            accepts_srv.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                while read_request_head(&mut sock).await {
                    let mut resp =
                        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n"
                            .to_vec();
                    for _ in 0..50 {
                        resp.extend_from_slice(b"A\r\n0123456789\r\n");
                    }
                    resp.extend_from_slice(b"0\r\n\r\n");
                    if sock.write_all(&resp).await.is_err() {
                        break;
                    }
                }
            });
        }
    });

    let session = Session::new();
    let r1 = session
        .request(http::Method::GET, format!("http://{addr}/a"))
        .stream()
        .send()
        .await
        .unwrap();
    let mut s1 = r1.into_stream().unwrap();
    let _first = s1.next().await.unwrap().unwrap();
    drop(s1);
    let r2 = session.get(format!("http://{addr}/b")).await.unwrap();
    assert_eq!(r2.status(), 200);
    assert_eq!(
        accepts.load(Ordering::SeqCst),
        2,
        "an early-dropped stream's connection must not be reused"
    );
}
