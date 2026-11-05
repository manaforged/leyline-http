use crate::{Browser, Platform, Proxy, Session};

#[test]
fn dropping_the_session_frees_its_identity_sessions() {
    let session = Session::builder()
        .browser(Browser::chrome())
        .proxy(Proxy::identified([
            ("http://127.0.0.1:9", Browser::chrome(), Platform::Windows),
            ("http://127.0.0.1:10", Browser::firefox(), Platform::Windows),
        ]))
        .build()
        .expect("session builds");
    let pool = session
        .proxy_pool()
        .expect("identified proxies make a pool")
        .clone();
    let lease = pool.lease(None, None).expect("a proxy is free");
    let derived = lease.session(&session).expect("identity session builds");
    let freed = derived.downgrade();
    drop((derived, lease, pool, session));
    assert_eq!(
        freed.strong_count(),
        0,
        "the identity session outlived its parent"
    );
}
