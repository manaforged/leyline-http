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

    assert_eq!(session.browser(), clone.browser());
    assert_eq!(session.platform(), clone.platform());
    assert_eq!(session.default_timeout(), clone.default_timeout());
}
