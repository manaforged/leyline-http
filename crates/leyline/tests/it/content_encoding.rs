#![cfg(feature = "compression-gzip")]

use leyline::{ProtocolPolicy, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn gzip(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

#[tokio::test]
async fn repeated_content_encoding_fields_decode_every_layer() {
    let body = gzip(&gzip(b"layered body"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        for _ in 0..2 {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            loop {
                let n = sock.read(&mut buf).await.unwrap();
                if n == 0 || buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\ncontent-encoding: gzip\r\n\
                 content-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(head.as_bytes()).await.unwrap();
            sock.write_all(&body).await.unwrap();
            sock.flush().await.unwrap();
        }
    });
    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();

    let buffered = session.get(format!("http://{addr}/b")).await.unwrap();
    assert_eq!(buffered.header("content-encoding"), None);
    assert_eq!(buffered.bytes().await.unwrap(), &b"layered body"[..]);

    let streamed = session
        .request(http::Method::GET, format!("http://{addr}/s"))
        .stream()
        .send()
        .await
        .unwrap();
    assert_eq!(streamed.bytes().await.unwrap(), &b"layered body"[..]);
}
