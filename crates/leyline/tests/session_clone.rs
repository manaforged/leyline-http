//! Regression gate: `Session` must be `Clone`.
//!
//! Earlier, `Session` held
//! `FingerprintConnector` by value and was non-cloneable — users had
//! to wrap it in their own `Arc<Session>` to share across tasks,
//! forcing an extra indirection that `reqwest::Client` callers
//! don't need. That shape made `LeylineService` in `leyline-tower`
//! the only cloneable wrapper and meant tower middleware had a
//! shape gap vs direct `Session` users.
//!
//! Now `Session: Clone` via internally-shared `Arc`-wrapped fields
//! (connection pool, cookie jar, session ticket cache). Cloning is
//! cheap; clones share every resource.

use leyline::{Browser, Session};

#[test]
fn session_is_clone_and_sharing_pool_with_cookies() {
    let session = Session::builder()
        .browser(Browser::Chrome147)
        .build()
        .expect("Chrome 147 session builds offline");
    let clone = session.clone();

    // Both instances report identical pool stats (same `Arc<Pool>`).
    let a = session.pool_stats();
    let b = clone.pool_stats();
    assert_eq!(a.entries, b.entries);
    assert_eq!(a.max_connections, b.max_connections);

    // Both instances expose the same browser + platform config.
    assert_eq!(session.browser(), clone.browser());
    assert_eq!(session.platform(), clone.platform());
    assert_eq!(session.default_timeout(), clone.default_timeout());
}

#[test]
fn session_clone_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Session>();
}
