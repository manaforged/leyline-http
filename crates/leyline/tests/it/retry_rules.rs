use futures_util::TryFutureExt;
use std::io::Write;
use std::time::Duration;

use bytes::Bytes;
use leyline::testing::{self, queue};
use leyline::{Body, Proxy, Retry, Session, Timeout};
use tokio::net::TcpListener;

async fn dead_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

fn plain() -> Session {
    Session::builder().retry(0).build().unwrap()
}

#[tokio::test]
async fn transient_returns_a_long_server_wait_instead_of_sleeping() {
    let server = testing::Server::http(queue([
        testing::Response::new(429)
            .close()
            .header("retry-after", "3600"),
        testing::Response::new(200).close(),
    ]))
    .await
    .unwrap();
    let session = Session::builder()
        .retry(3)
        .timeout(Timeout::new().total(None))
        .build()
        .unwrap();
    let resp = tokio::time::timeout(Duration::from_secs(5), session.get(server.url("/")).send())
        .await
        .expect("slept for the server wait")
        .unwrap();
    assert_eq!(resp.status().as_u16(), 429);
    assert_eq!(server.requests().await.len(), 1);
}

#[tokio::test]
async fn a_configured_wait_header_wins_over_retry_after() {
    let server = testing::Server::http(queue([
        testing::Response::new(429)
            .close()
            .header("retry-after", "100")
            .header("x-wait", "0"),
        testing::Response::new(200).close(),
    ]))
    .await
    .unwrap();
    let session = Session::builder()
        .retry(
            Retry::new(3)
                .max_wait(Duration::from_secs(5))
                .wait_header("x-wait"),
        )
        .build()
        .unwrap();
    let resp = session.get(server.url("/")).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(server.requests().await.len(), 2);
}

#[tokio::test]
async fn a_millisecond_wait_header_waits_milliseconds() {
    let server = testing::Server::http(queue([
        testing::Response::new(429)
            .close()
            .header("x-wait-ms", "50"),
        testing::Response::new(200).close(),
    ]))
    .await
    .unwrap();
    let session = Session::builder()
        .retry(
            Retry::new(3)
                .max_wait(Duration::from_secs(1))
                .wait_header_ms("x-wait-ms"),
        )
        .build()
        .unwrap();
    let resp = session.get(server.url("/")).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(server.requests().await.len(), 2);
}

#[tokio::test]
async fn a_declined_retry_counts_as_exhausted() {
    let server = testing::Server::http(queue([testing::Response::new(503)
        .close()
        .header("retry-after", "100")]))
    .await
    .unwrap();
    let err = Session::builder()
        .retry(Retry::new(3).max_wait(Duration::from_secs(1)))
        .build()
        .unwrap()
        .get(server.url("/"))
        .error_for_status()
        .send()
        .await
        .unwrap_err();
    assert_eq!(err.status().map(|s| s.as_u16()), Some(503));
    assert!(err.retries_exhausted());
}

#[tokio::test]
async fn an_unsent_streaming_body_is_retried() {
    let proxy = testing::Server::http(queue([testing::Response::new(200).close()]))
        .await
        .unwrap();
    let session = Session::builder()
        .proxy(Proxy::list([
            format!("http://127.0.0.1:{}", dead_port().await),
            format!("http://{}", proxy.addr()),
        ]))
        .retry(Retry::new(3).backoff(Duration::from_millis(1), Duration::from_secs(1)))
        .build()
        .unwrap();
    let chunks = futures_util::stream::iter([
        Ok::<_, std::io::Error>(Bytes::from_static(b"part-one,")),
        Ok(Bytes::from_static(b"part-two")),
    ]);
    let resp = session
        .post("http://origin.test/upload")
        .body(Body::stream(chunks, None))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let sent = proxy.next_request().await.unwrap();
    assert_eq!(sent.target, "http://origin.test/upload");
    assert_eq!(sent.body, b"part-one,part-two");
}

#[tokio::test]
async fn download_keeps_the_same_error_body_as_send() {
    let body = vec![b'e'; 10 * 1024];
    let server = testing::Server::http(queue([testing::Response::new(503)
        .close()
        .body(body.clone())]))
    .await
    .unwrap();
    let dir = std::env::temp_dir().join(format!("leyline-errbody-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let err = plain()
        .get(server.url("/file"))
        .error_for_status()
        .stream()
        .send()
        .and_then(|resp| resp.download_to(dir.join("file.bin")))
        .await
        .unwrap_err();
    assert_eq!(err.body(), Some(body.as_slice()));
    drop(std::fs::remove_dir_all(&dir));
}

#[tokio::test]
async fn an_error_body_bomb_stops_at_the_error_body_cap() {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(&vec![0u8; 4 << 20]).unwrap();
    let bomb = encoder.finish().unwrap();
    let server = testing::Server::http(queue([testing::Response::new(503)
        .close()
        .header("content-encoding", "gzip")
        .body(bomb)]))
    .await
    .unwrap();
    let dir = std::env::temp_dir().join(format!("leyline-bomb-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let err = Session::builder()
        .retry(0)
        .max_body_size(1 << 20)
        .build()
        .unwrap()
        .get(server.url("/"))
        .error_for_status()
        .stream()
        .send()
        .and_then(|resp| resp.download_to(dir.join("bomb.bin")))
        .await
        .unwrap_err();
    drop(std::fs::remove_dir_all(&dir));
    assert_eq!(err.status().map(|s| s.as_u16()), Some(503));
    assert_eq!(err.body().map(<[u8]>::len), Some(64 * 1024));
}

#[tokio::test]
async fn the_body_timeout_starts_at_the_first_read() {
    let server = testing::Server::http(queue([testing::Response::new(200).close().body("fast")]))
        .await
        .unwrap();
    let resp = plain()
        .get(server.url("/"))
        .timeout(Timeout::new().body(Duration::from_millis(300)))
        .stream()
        .send()
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(resp.bytes().await.unwrap(), "fast");
}

#[tokio::test]
async fn a_hostile_retry_after_year_is_ignored() {
    let server = testing::Server::http(queue([
        testing::Response::new(503)
            .close()
            .header("retry-after", "Mon, 01 Jan -2147483648 00:00:00 GMT"),
        testing::Response::new(200).close(),
    ]))
    .await
    .unwrap();
    let resp = Session::builder()
        .retry(Retry::new(3).backoff(Duration::from_millis(1), Duration::from_secs(1)))
        .build()
        .unwrap()
        .get(server.url("/"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
}

async fn error_body_len(gzip: bool) -> Option<usize> {
    let payload = vec![b'e'; 20 * 1024];
    let mut response = testing::Response::new(500).close();
    response = if gzip {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&payload).unwrap();
        response
            .header("content-encoding", "gzip")
            .body(encoder.finish().unwrap())
    } else {
        response.body(payload)
    };
    let server = testing::Server::http(queue([response])).await.unwrap();
    let err = Session::builder()
        .retry(0)
        .max_body_size(16 * 1024)
        .build()
        .unwrap()
        .get(server.url("/"))
        .stream()
        .error_for_status()
        .send()
        .await
        .unwrap_err();
    err.body().map(<[u8]>::len)
}

#[tokio::test]
async fn an_error_body_is_cut_at_the_session_cap_with_or_without_encoding() {
    assert_eq!(error_body_len(false).await, Some(16 * 1024));
    assert_eq!(error_body_len(true).await, Some(16 * 1024));
}

#[test]
fn a_retry_gives_its_backoff_wait_for_an_outside_loop() {
    let retry =
        leyline::Retry::new(3).backoff(Duration::from_millis(100), Duration::from_millis(400));
    for (attempt, ceiling) in [(0, 100), (1, 200), (2, 400), (5, 400)] {
        let wait = retry.delay(attempt);
        let ceiling = Duration::from_millis(ceiling);
        assert!(
            wait >= ceiling / 2 && wait <= ceiling,
            "{attempt}: {wait:?}"
        );
    }
}
