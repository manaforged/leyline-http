//! Regression test for the H2 coalesced-connect FAILURE path.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use leyline::h2::H2Config;
use leyline::pool::Pool;
use leyline::profile::{Browser, Platform, ProfileRegistry};
use leyline::tls::FingerprintConnector;
use tokio::net::TcpListener;

fn bare_connector() -> FingerprintConnector {
    let registry = ProfileRegistry::builtin();
    let profile = registry
        .get_browser(Browser::Chrome147)
        .expect("chrome147 profile is bundled");
    FingerprintConnector::new(profile, Platform::Windows.tcp_profile())
        .expect("build fingerprint connector")
}

fn chrome_h2_config() -> H2Config {
    let registry = ProfileRegistry::builtin();
    let profile = registry
        .get_browser(Browser::Chrome147)
        .expect("chrome147 profile is bundled");
    H2Config::from_profile(&profile.h2).expect("valid built-in h2 profile")
}

#[tokio::test]
async fn coalesced_h2_connect_failure_shares_one_retry() {
    const WAITERS: usize = 20;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accepts = Arc::new(AtomicUsize::new(0));
    let accepts_c = accepts.clone();
    let _server = tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                break;
            };
            accepts_c.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(150)).await;
                drop(socket);
            });
        }
    });

    let pool = Arc::new(Pool::new());
    let connector = bare_connector();
    let h2_config = chrome_h2_config();
    let host = "127.0.0.1".to_string();
    let port = addr.port();

    let mut handles = Vec::new();
    for _ in 0..WAITERS {
        let pool = Arc::clone(&pool);
        let connector = connector.clone();
        let h2_config = h2_config.clone();
        let host = host.clone();
        handles.push(tokio::spawn(async move {
            leyline::pool::checkout_handle(&pool, &connector, &h2_config, &host, port, None).await
        }));
    }

    for h in handles {
        let r = tokio::time::timeout(Duration::from_secs(10), h)
            .await
            .expect("checkout task did not hang")
            .expect("checkout task did not panic");
        let err = match r {
            Ok(_) => panic!("connect to a handshake-failing host must surface an error"),
            Err(err) => err,
        };
        assert!(
            matches!(err.tls(), Some(leyline::tls::TlsError::HandshakeIo(_))),
            "coalescing must preserve the typed TLS handshake I/O failure, got {err:?}"
        );
    }

    tokio::time::sleep(Duration::from_millis(50)).await;
    let dials = accepts.load(Ordering::SeqCst);

    assert!(
        dials <= 5,
        "coalesced connect failure fanned out to {dials} dials \
         (expected ~2; a storm would be ~{})",
        WAITERS + 1
    );
}
