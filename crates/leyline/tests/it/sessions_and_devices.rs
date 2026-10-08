use std::time::Duration;

use leyline::multipart::{Form, Part};
use leyline::testing::{TestResponse, TestServer, queue};
use leyline::{BlockRules, Browser, Device, Kind, ProxyPool, ProxyUrl, RetryPolicy, Session};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("leyline-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn user_agent_replaces_the_default_once() {
    let server = TestServer::http(queue(vec![
        TestResponse::new(200).body("ok").close(),
        TestResponse::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();
    for builder in [
        Session::builder(),
        Session::builder().browser(Browser::default()),
    ] {
        let session = builder.user_agent("my-tool/1.0").build().unwrap();
        session.get(server.url("/")).await.unwrap();
        let sent = server.next_request().await.unwrap();
        let agents = sent.header_values("user-agent");
        assert_eq!(agents, ["my-tool/1.0"]);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_stops_requests_in_flight_and_new_ones() {
    let server = TestServer::http(|_| TestResponse::new(200).delay(Duration::from_secs(5)))
        .await
        .unwrap();
    let session = Session::new();
    let clone = session.clone();
    let url = server.url("/slow");
    let pending = tokio::spawn({
        let clone = clone.clone();
        let url = url.clone();
        async move { clone.get(url).send().await }
    });
    server.next_request().await.unwrap();
    session.shutdown();
    let err = tokio::time::timeout(Duration::from_secs(1), pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Request);
    assert!(clone.is_shut_down());
    assert_eq!(clone.get(url).await.unwrap_err().kind(), Kind::Request);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_test_server_reports_a_request_before_its_delayed_reply() {
    let server = TestServer::http(|_| TestResponse::new(200).delay(Duration::from_secs(5)))
        .await
        .unwrap();
    let url = server.url("/slow");
    let pending = tokio::spawn(async move { Session::new().get(url).send().await });
    let seen = tokio::time::timeout(Duration::from_secs(2), server.next_request()).await;
    assert!(matches!(seen, Ok(Some(_))), "{seen:?}");
    pending.abort();
}

#[tokio::test]
async fn skip_blocks_returns_a_block_without_retrying() {
    let server = TestServer::http(queue(vec![
        TestResponse::new(429).close(),
        TestResponse::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();
    let session = Session::builder()
        .retry(
            RetryPolicy::transient()
                .initial_backoff(Duration::from_millis(1))
                .skip_blocks(BlockRules::statuses([429])),
        )
        .build()
        .unwrap();
    let resp = session.get(server.url("/")).await.unwrap();
    assert_eq!(resp.status().as_u16(), 429);
    assert_eq!(resp.attempts(), 1);
}

#[tokio::test]
async fn rotate_on_block_moves_the_origin_to_another_proxy() {
    let first = TestServer::http(queue(vec![
        TestResponse::new(200)
            .body("ok")
            .close()
            .header("cf-mitigated", "challenge"),
    ]))
    .await
    .unwrap();
    let second = TestServer::http(queue(vec![TestResponse::new(200).body("ok").close()]))
        .await
        .unwrap();
    let pool = ProxyPool::new([
        format!("http://{}", first.addr()),
        format!("http://{}", second.addr()),
    ])
    .rotate_on_block(BlockRules::builtin().clone());
    let session = Session::builder().proxy_pool(pool).build().unwrap();
    session.get("http://origin.test/a").await.unwrap();
    session.get("http://origin.test/b").await.unwrap();
    assert_eq!(
        second.next_request().await.unwrap().request_line,
        "GET http://origin.test/b HTTP/1.1"
    );
}

#[tokio::test]
async fn retry_unsent_retries_a_post_that_never_left() {
    let policy = RetryPolicy::transient()
        .max_retries(2)
        .initial_backoff(Duration::from_millis(1));
    let post = |policy: RetryPolicy| async move {
        Session::builder()
            .retry(policy)
            .build()
            .unwrap()
            .request(leyline::http::Method::POST, "http://127.0.0.1:9/")
            .body("x")
            .await
            .unwrap_err()
            .attempts()
    };
    assert_eq!(post(policy.clone()).await, 1);
    assert_eq!(post(policy.retry_unsent(true)).await, 3);
}

#[test]
fn a_device_keeps_the_proxy_password_out_of_its_file() {
    let dir = scratch("devsecret");
    let session = Session::browser(Browser::Chrome154);
    let mut device = Device::capture(&session, None);
    device.proxy = Some(ProxyUrl::parse("http://user:hunter2@proxy.example:8080").unwrap());
    device.proxy_password_env = Some("LEYLINE_TEST_PROXY_PASSWORD".to_owned());
    let path = dir.join("device.json");
    device.save_to(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("hunter2"), "{text}");

    let saved = Device::load_from(&path).unwrap();
    let err = saved.open().unwrap_err();
    assert_eq!(err.kind(), Kind::Config);
    assert!(err.to_string().contains("LEYLINE_TEST_PROXY_PASSWORD"));
    drop(std::fs::remove_dir_all(&dir));
}

#[tokio::test]
async fn a_file_part_takes_a_mime_type() {
    let dir = scratch("partfile");
    let path = dir.join("logo.png");
    std::fs::write(&path, b"png-bytes").unwrap();
    let server = TestServer::http(|_| TestResponse::new(200)).await.unwrap();
    let form = Form::new().part("logo", Part::file(&path).unwrap().mime("image/png"));
    Session::new()
        .post(server.url("/upload"))
        .multipart(form)
        .await
        .unwrap();
    let body = String::from_utf8(server.requests().await[0].body.clone()).unwrap();
    assert!(body.contains("filename=\"logo.png\""), "{body}");
    assert!(
        body.contains("Content-Type: image/png") || body.contains("content-type: image/png"),
        "{body}"
    );
    assert!(body.contains("png-bytes"));
    drop(std::fs::remove_dir_all(&dir));
}
