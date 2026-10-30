use std::time::Duration;

use leyline::multipart::{Form, Part};
use leyline::testing::{self, queue};
use leyline::{Browser, Kind, Proxy, Retry, Session};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("leyline-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn user_agent_replaces_the_default_once() {
    let server = testing::Server::http(queue(vec![
        testing::Response::new(200).body("ok").close(),
        testing::Response::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();
    for builder in [
        Session::builder(),
        Session::builder().browser(Browser::default()),
    ] {
        let session = builder
            .headers([("user-agent", "my-tool/1.0")])
            .build()
            .unwrap();
        session.get(server.url("/")).send().await.unwrap();
        let sent = server.next_request().await.unwrap();
        let agents = sent.header_values("user-agent");
        assert_eq!(agents, ["my-tool/1.0"]);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_stops_requests_in_flight_and_new_ones() {
    let server =
        testing::Server::http(|_| testing::Response::new(200).delay(Duration::from_secs(5)))
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
    session.shutdown().await;
    let err = tokio::time::timeout(Duration::from_secs(1), pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Request);
    assert!(clone.is_shut_down());
    assert_eq!(
        clone.get(url).send().await.unwrap_err().kind(),
        Kind::Request
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_test_server_reports_a_request_before_its_delayed_reply() {
    let server =
        testing::Server::http(|_| testing::Response::new(200).delay(Duration::from_secs(5)))
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
    let server = testing::Server::http(queue(vec![
        testing::Response::new(429).close(),
        testing::Response::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();
    let session = Session::builder()
        .block_statuses([429])
        .retry(
            Retry::new(3)
                .backoff(Duration::from_millis(1), Duration::from_secs(1))
                .when(|attempt| attempt.is_transient() && !attempt.is_block()),
        )
        .build()
        .unwrap();
    let resp = session.get(server.url("/")).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 429);
    assert_eq!(resp.attempts(), 1);
}

#[tokio::test]
async fn a_block_moves_the_origin_to_another_proxy() {
    let first = testing::Server::http(queue(vec![
        testing::Response::new(200)
            .body("ok")
            .close()
            .header("cf-mitigated", "challenge"),
    ]))
    .await
    .unwrap();
    let second = testing::Server::http(queue(vec![testing::Response::new(200).body("ok").close()]))
        .await
        .unwrap();
    let pool = Proxy::list([
        format!("http://{}", first.addr()),
        format!("http://{}", second.addr()),
    ])
    .sticky_for(Duration::from_secs(60));
    let session = Session::builder().proxy(pool).build().unwrap();
    session.get("http://origin.test/a").send().await.unwrap();
    let moved = tokio::time::timeout(Duration::from_secs(5), async {
        session.get("http://origin.test/b").send().await.unwrap();
        second.next_request().await.unwrap().request_line
    })
    .await
    .expect("the challenge did not move the origin off the first proxy");
    assert_eq!(moved, "GET http://origin.test/b HTTP/1.1");
}

#[tokio::test]
async fn a_post_that_never_left_retries_by_default() {
    let attempts = Session::builder()
        .retry(Retry::new(2).backoff(Duration::from_millis(1), Duration::from_millis(5)))
        .build()
        .unwrap()
        .request(leyline::http::Method::POST, "http://127.0.0.1:9/")
        .body("x")
        .send()
        .await
        .unwrap_err()
        .attempts();
    assert_eq!(attempts, 3);
}

#[test]
fn a_device_keeps_the_proxy_password_out_of_its_file() {
    let dir = scratch("devsecret");
    let proxy = "http://user:hunter2@proxy.example:8080";
    let session = Session::builder()
        .browser(Browser::Chrome154)
        .proxy(proxy)
        .build()
        .unwrap();
    let path = dir.join("device.json");
    session.save(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("hunter2"), "{text}");
    assert!(text.contains("proxy.example"), "{text}");

    let reopened = Session::load(&path).unwrap().proxy(proxy).build().unwrap();
    let shown = reopened.proxy().unwrap();
    assert!(shown.contains("proxy.example"), "{shown}");
    assert!(!shown.contains("hunter2"), "{shown}");
    drop(std::fs::remove_dir_all(&dir));
}

#[tokio::test]
async fn a_file_part_takes_a_mime_type() {
    let dir = scratch("partfile");
    let path = dir.join("logo.png");
    std::fs::write(&path, b"png-bytes").unwrap();
    let server = testing::Server::http(|_| testing::Response::new(200))
        .await
        .unwrap();
    let form = Form::new().part("logo", Part::file(&path).unwrap().mime("image/png"));
    Session::new()
        .post(server.url("/upload"))
        .multipart(form)
        .send()
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

#[tokio::test]
async fn one_block_rule_set_strikes_the_proxy_and_skips_the_retry() {
    let first = testing::Server::http(queue(vec![testing::Response::new(403).close()]))
        .await
        .unwrap();
    let second = testing::Server::http(queue(vec![testing::Response::new(200).body("ok").close()]))
        .await
        .unwrap();
    let pool = Proxy::list([
        format!("http://{}", first.addr()),
        format!("http://{}", second.addr()),
    ])
    .ban_after(1);
    let session = Session::builder()
        .proxy(pool.clone())
        .block_statuses([403])
        .retry(
            Retry::new(3)
                .backoff(Duration::from_millis(1), Duration::from_secs(1))
                .when(|attempt| {
                    (attempt.is_transient() || attempt.status().is_some_and(|s| s.as_u16() == 403))
                        && !attempt.is_block()
                }),
        )
        .build()
        .unwrap();
    let blocked = session.get("http://origin.test/a").send().await.unwrap();
    assert_eq!(blocked.status().as_u16(), 403);
    assert_eq!(blocked.attempts(), 1);
    let stats = session.stats();
    assert!(
        stats
            .proxy_banned_until(stats.proxies().next().unwrap())
            .is_some_and(|until| until > std::time::Instant::now())
    );
    let next = session.get("http://origin.test/b").send().await.unwrap();
    assert_eq!(next.status().as_u16(), 200);
}

#[tokio::test]
async fn a_derived_pool_rotates_only_the_derived_session() {
    let dead = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_port = dead.local_addr().unwrap().port();
    drop(dead);
    let good = testing::Server::http(queue(vec![testing::Response::new(200).body("ok").close()]))
        .await
        .unwrap();
    let parent = Session::builder()
        .proxy(format!("http://127.0.0.1:{dead_port}"))
        .retry(Retry::new(3).backoff(Duration::from_millis(1), Duration::from_secs(1)))
        .build()
        .unwrap();
    let rotating = parent
        .to_builder()
        .proxy(Proxy::list([
            format!("http://127.0.0.1:{dead_port}"),
            format!("http://{}", good.addr()),
        ]))
        .build()
        .unwrap();
    let resp = rotating
        .get("http://origin.test/item")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert!(
        parent
            .get("http://origin.test/item")
            .send()
            .await
            .unwrap_err()
            .is_proxy()
    );
}
