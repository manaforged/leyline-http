use leyline::{Browser, Session};

#[test]
fn session_is_clone_and_sharing_pool_with_cookies() {
    let session = Session::builder()
        .browser(Browser::Chrome147)
        .build()
        .expect("Chrome 147 session builds offline");
    let clone = session.clone();

    let a = session.pool_stats();
    let b = clone.pool_stats();
    assert_eq!(a.entries, b.entries);
    assert_eq!(a.max_connections, b.max_connections);

    assert_eq!(format!("{session}"), format!("{clone}"));
}

#[test]
fn session_reports_the_exit_it_is_bound_to() {
    let session = Session::builder()
        .browser(Browser::Chrome147)
        .build()
        .expect("Chrome 147 session builds offline");
    assert!(session.proxy_url().is_none());
    let bound = session.with_proxy("http://user:pw@127.0.0.1:9000");
    assert_eq!(
        bound.proxy_url().map(String::from).as_deref(),
        Some("http://user:pw@127.0.0.1:9000")
    );
}
