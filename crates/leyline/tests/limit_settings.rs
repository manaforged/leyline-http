use std::time::Duration;

use leyline::testing::{TestResponse, TestServer, queue};
use leyline::{CompressionConfig, HostLimits, RetryPolicy, Session};

#[tokio::test]
async fn max_pause_caps_a_long_server_wait() {
    let server = TestServer::http(queue([
        TestResponse::new(429).close().header("retry-after", "3600"),
        TestResponse::new(200).close(),
    ]))
    .await
    .unwrap();
    let session = Session::builder()
        .host_limits(
            HostLimits::new()
                .pause_on([429])
                .max_pause(Duration::from_millis(200)),
        )
        .build()
        .unwrap();
    session.get(server.url("/")).await.unwrap();
    let next = tokio::time::timeout(Duration::from_secs(3), session.get(server.url("/")).send())
        .await
        .expect("the pause was not capped")
        .unwrap();
    assert_eq!(next.status().as_u16(), 200);
}

#[tokio::test]
async fn max_error_body_sets_the_kept_body() {
    let server = TestServer::http(queue([TestResponse::new(500)
        .close()
        .body(vec![b'x'; 4096])]))
    .await
    .unwrap();
    let err = Session::builder()
        .retry(RetryPolicy::none())
        .compression(CompressionConfig::new().max_error_body(100))
        .build()
        .unwrap()
        .get(server.url("/"))
        .error_for_status()
        .send()
        .await
        .unwrap_err();
    assert_eq!(err.body().map(<[u8]>::len), Some(100));
}

#[tokio::test]
async fn pages_limit_stops_an_endless_pagination() {
    let served = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = std::sync::Arc::clone(&served);
    let server = TestServer::http(move |_| {
        let n = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        TestResponse::new(200)
            .close()
            .header("link", format!("</items?page={}>; rel=\"next\"", n + 2))
    })
    .await
    .unwrap();
    let mut pages = Session::new().get(server.url("/items")).pages().limit(3);
    let mut count = 0;
    while let Some(page) = pages.next().await {
        page.unwrap();
        count += 1;
    }
    assert_eq!(count, 3);
    assert_eq!(server.requests().await.len(), 3);
}
