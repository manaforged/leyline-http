use leyline::{Browser, Session};

#[test]
fn session_is_clone_and_sharing_pool_with_cookies() {
    let session = Session::builder()
        .browser(Browser::Chrome147)
        .build()
        .expect("Chrome 147 session builds offline");
    let clone = session.clone();

    let a = session.stats();
    let b = clone.stats();
    assert_eq!(a.entries(), b.entries());
    assert_eq!(a.max_connections(), b.max_connections());

    assert_eq!(format!("{session}"), format!("{clone}"));
}

#[test]
fn session_reports_the_exit_it_is_bound_to() {
    let session = Session::builder()
        .browser(Browser::Chrome147)
        .build()
        .expect("Chrome 147 session builds offline");
    assert!(session.proxy().is_none());
    let bound = session
        .to_builder()
        .proxy("http://user:pw@127.0.0.1:9000")
        .build()
        .unwrap();
    let shown = bound.proxy().unwrap();
    assert!(shown.contains("127.0.0.1:9000"), "{shown}");
    assert!(!shown.contains(":pw@"), "{shown}");
}

#[tokio::test]
async fn to_builder_shares_the_pool_until_a_connection_setting_changes() {
    let server = leyline::testing::Server::http(|_| leyline::testing::Response::new(200))
        .await
        .unwrap();
    let parent = Session::new();
    parent.get(server.url("/warm")).send().await.unwrap();

    let headed = parent
        .to_builder()
        .headers([("x-child", "1")])
        .build()
        .unwrap();
    let reused = headed.get(server.url("/headed")).send().await.unwrap();
    assert!(reused.reused());
    assert_eq!(server.next_request().await.unwrap().header("x-child"), None);
    assert_eq!(
        server.next_request().await.unwrap().header("x-child"),
        Some("1")
    );

    let separate = parent
        .to_builder()
        .share_connections(false)
        .build()
        .unwrap();
    let fresh = separate.get(server.url("/separate")).send().await.unwrap();
    assert!(!fresh.reused());

    parent.shutdown();
    assert!(headed.is_shut_down() && separate.is_shut_down());
}
