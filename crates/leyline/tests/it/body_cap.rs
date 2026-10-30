#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use std::time::Duration;

use futures_util::StreamExt;
use leyline::{Kind, Session, Timeout};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const CAP: usize = 1024;
const LENGTH: usize = 4096;
const FIRST_CHUNK: usize = CAP - 8;
const SECOND_CHUNK: usize = 16;
const STALL: Duration = Duration::from_secs(30);

async fn fixed_length_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 4096];
            drop(socket.read(&mut buf).await);
            let head =
                format!("HTTP/1.1 200 OK\r\nContent-Length: {LENGTH}\r\nConnection: close\r\n\r\n");
            drop(socket.write_all(head.as_bytes()).await);
            drop(socket.write_all(&[b'x'; LENGTH]).await);
        }
    });
    base
}

async fn stalled_chunked_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut buf = [0u8; 4096];
        drop(socket.read(&mut buf).await);
        let head =
            format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{FIRST_CHUNK:x}\r\n");
        drop(socket.write_all(head.as_bytes()).await);
        drop(socket.write_all(&[b'x'; FIRST_CHUNK]).await);
        let next = format!("\r\n{SECOND_CHUNK:x}\r\n");
        drop(socket.write_all(next.as_bytes()).await);
        tokio::time::sleep(STALL).await;
    });
    base
}

fn capped() -> Session {
    Session::builder().max_body_size(CAP).build().unwrap()
}

#[tokio::test]
async fn a_buffered_body_over_the_cap_is_a_body_error() {
    let base = fixed_length_server().await;
    let session = capped();
    let result = async { session.get(&base).send().await?.bytes().await }.await;
    let err = result.unwrap_err();
    assert_eq!(err.kind(), Kind::BodyLimit, "{err:?}");
}

#[tokio::test]
async fn a_streamed_body_stops_at_the_cap_unless_the_request_raises_it() {
    let base = fixed_length_server().await;
    let session = capped();
    let mut capped = Vec::new();
    let err = session
        .get(&base)
        .stream()
        .send()
        .await
        .unwrap()
        .write_to(&mut capped)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), Kind::BodyLimit, "{err:?}");
    let base = fixed_length_server().await;
    let mut body = session
        .get(&base)
        .max_body_size(LENGTH)
        .stream()
        .send()
        .await
        .unwrap()
        .bytes_stream()
        .unwrap();
    let mut read = 0;
    while let Some(chunk) = body.next().await {
        read += chunk.unwrap().len();
    }
    assert_eq!(read, LENGTH);
}

#[tokio::test]
async fn a_chunk_that_would_pass_the_cap_fails_before_its_bytes_arrive() {
    let base = stalled_chunked_server().await;
    let session = Session::builder()
        .max_body_size(CAP)
        .timeout(Timeout::new().total(Duration::from_secs(2)))
        .build()
        .unwrap();
    let result = async { session.get(&base).send().await?.bytes().await }.await;
    let err = result.unwrap_err();
    assert_eq!(err.kind(), Kind::BodyLimit, "{err:?}");
}

#[tokio::test]
async fn a_corrupt_compressed_body_fails_before_the_rest_arrives() {
    use leyline::testing::{self, queue};
    let server = testing::Server::http(queue([testing::Response::new(200)
        .header("content-encoding", "gzip")
        .chunks([vec![0u8; 64], vec![0u8; 64]], STALL)]))
    .await
    .unwrap();
    let response = Session::builder()
        .build()
        .unwrap()
        .get(server.url("/"))
        .stream()
        .send()
        .await
        .unwrap();
    let error = tokio::time::timeout(Duration::from_secs(5), response.bytes())
        .await
        .expect("the decoder must fail on the first chunk, not wait for the stalled one")
        .unwrap_err();
    assert_eq!(error.kind(), Kind::Decode);
}
