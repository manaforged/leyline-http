use futures_util::StreamExt;
use std::time::Duration;

use leyline::testing::{self, queue};
use leyline::{Block, Browser, Error, Kind, Platform, Proxy, Session};

fn plain() -> Session {
    Session::new()
}

#[tokio::test]
async fn pages_follow_next_links_and_stop_on_a_loop() {
    let server = testing::Server::http(|req| match req.target.as_str() {
        "/items" => testing::Response::new(200)
            .header("link", "</items?page=2>; rel=\"next\"")
            .body("one"),
        "/items?page=2" => testing::Response::new(200)
            .header("link", "</items>; rel=\"next\"")
            .body("two"),
        _ => testing::Response::new(404),
    })
    .await
    .unwrap();
    let mut pages = plain()
        .get(server.url("/items"))
        .header("x-client", "pager")
        .pages();
    let mut bodies = Vec::new();
    while let Some(page) = pages.next().await {
        bodies.push(page.unwrap().text().await.unwrap());
    }
    assert_eq!(bodies, ["one", "two"]);
    let requests = server.requests().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].header("x-client"), Some("pager"));
}

#[tokio::test]
async fn a_status_error_gives_its_wait_and_body_text() {
    let server = testing::Server::http(queue(vec![
        testing::Response::new(429)
            .close()
            .header("retry-after", "7")
            .body(b"slow down".to_vec()),
    ]))
    .await
    .unwrap();
    let err = plain()
        .get(server.url("/"))
        .error_for_status()
        .send()
        .await
        .unwrap_err();
    assert_eq!(err.retry_after(), Some(Duration::from_secs(7)));
    assert_eq!(
        err.body().map(String::from_utf8_lossy).as_deref(),
        Some("slow down")
    );
}

#[derive(Debug)]
struct Layer(Error);

impl std::fmt::Display for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("service layer")
    }
}

impl std::error::Error for Layer {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

#[tokio::test]
async fn find_reaches_a_leyline_error_inside_other_layers() {
    let err = plain().get("http://127.0.0.1:9/").send().await.unwrap_err();
    let boxed: Box<dyn std::error::Error + Send + Sync> = Box::new(Layer(err));
    let found = Error::find(boxed.as_ref()).unwrap();
    assert_eq!(found.kind(), Kind::Connect);
}

#[tokio::test]
async fn identified_gives_each_proxy_its_identity() {
    let first = testing::Server::http(queue(vec![testing::Response::new(403).close()]))
        .await
        .unwrap();
    let second = testing::Server::http(queue(vec![testing::Response::new(200).body("ok").close()]))
        .await
        .unwrap();
    let pool = Proxy::identified([
        (
            format!("http://{}", first.addr()),
            Browser::chrome(),
            Platform::Windows,
        ),
        (
            format!("http://{}", second.addr()),
            Browser::firefox(),
            Platform::Windows,
        ),
    ]);
    let session = Session::builder()
        .browser(Browser::chrome())
        .proxy(pool)
        .block_statuses([403])
        .build()
        .unwrap();
    session.get("http://origin.test/a").send().await.unwrap();
    session.get("http://origin.test/b").send().await.unwrap();
    let sent = second.next_request().await.unwrap();
    let agent = sent.header_values("user-agent");
    assert!(agent[0].contains("Firefox"), "{agent:?}");
}

#[tokio::test]
async fn status_rules_turn_a_bare_status_into_a_block() {
    let server = testing::Server::http(queue(vec![testing::Response::new(429).close(); 2]))
        .await
        .unwrap();
    let resp = plain().get(server.url("/")).send().await.unwrap();
    assert!(resp.block().is_none());
    let session = Session::builder()
        .block_statuses([403, 429])
        .build()
        .unwrap();
    let resp = session.get(server.url("/")).send().await.unwrap();
    assert_eq!(resp.block(), Some(&Block::Denied("status".into())));
}
