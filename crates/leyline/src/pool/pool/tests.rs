use super::{OriginFlag, POOL_REAP_INTERVAL, Pool};
use crate::Session;
use crate::pool::types::PoolKey;
use crate::profile::Carrier;
use crate::testing::{Response, Server, queue};
use crate::wire::TlsInfo;
use std::time::{Duration, Instant};

fn h1_key(host: &str) -> PoolKey {
    PoolKey {
        scheme: crate::wire::Scheme::Http,
        host: host.to_string(),
        port: 80,
        proxy: None,
        transport: Carrier::Tcp,
        partition: None,
        lane: 0,
    }
}

async fn fetch(session: &Session, url: String) -> String {
    let response = session.get(url).send().await.expect("response");
    response.text().await.expect("body")
}

async fn empty_h1_entry(idle_timeout: Duration) -> (Session, Server) {
    let server = Server::http(queue([
        Response::new(200).body("first"),
        Response::new(200).close().body("last"),
    ]))
    .await
    .expect("server");
    let session = Session::builder()
        .pool_idle_timeout(idle_timeout)
        .build()
        .expect("session");
    assert_eq!(fetch(&session, server.url("/")).await, "first");
    let last = session.get(server.url("/")).send().await.expect("response");
    assert!(last.reused(), "the second request takes the pooled socket");
    assert_eq!(last.text().await.expect("body"), "last");
    assert_eq!(session.stats().entries(), 1);
    (session, server)
}

async fn ok_server() -> Server {
    Server::http(|_| Response::new(200).body("ok"))
        .await
        .expect("server")
}

#[tokio::test]
async fn evict_idle_drops_abandoned_empty_h1_entry() {
    let (session, _server) = empty_h1_entry(Duration::from_millis(500)).await;
    let other = ok_server().await;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        assert_eq!(fetch(&session, other.url("/")).await, "ok");
        if session.stats().entries() == 1 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "an empty H1 entry idle past the timeout must be reaped, not linger until LRU"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn evict_idle_keeps_empty_h1_entry_inside_the_idle_window() {
    let (session, _server) = empty_h1_entry(Duration::from_secs(300)).await;
    let other = ok_server().await;
    tokio::time::sleep(POOL_REAP_INTERVAL + Duration::from_millis(50)).await;

    assert_eq!(fetch(&session, other.url("/")).await, "ok");

    assert_eq!(
        session.stats().entries(),
        2,
        "an empty H1 entry touched within the idle window must survive the reap"
    );
}

#[cfg(feature = "http3")]
#[test]
fn alt_svc_marks_h3_origin() {
    let pool = Pool::new();
    assert!(!pool.knows_h3("example.com", 443));
    pool.note_alt_svc(
        "example.com",
        443,
        &["h2=\":443\"; ma=86400"],
        Duration::ZERO,
    );
    assert!(!pool.knows_h3("example.com", 443));
    pool.note_alt_svc(
        "example.com",
        443,
        &["h3=\":443\"; ma=86400, h3-29=\":443\""],
        Duration::ZERO,
    );
    assert!(pool.knows_h3("example.com", 443));
    assert!(!pool.knows_h3("example.com", 8443));
    pool.note_alt_svc("other.example", 443, &["h3=\":8443\""], Duration::ZERO);
    assert!(!pool.knows_h3("other.example", 443));
    pool.note_alt_svc(
        "cross.example",
        443,
        &["h3=\"elsewhere.example:443\""],
        Duration::ZERO,
    );
    assert!(!pool.knows_h3("cross.example", 443));
    pool.note_alt_svc(
        "same.example",
        443,
        &["h3=\"same.example:443\""],
        Duration::ZERO,
    );
    assert!(pool.knows_h3("same.example", 443));
}

#[test]
fn alpn_h1_memory_is_per_origin_and_proxy() {
    let pool = Pool::new();
    assert!(!pool.recalls(OriginFlag::H1Only, "example.com", 443, None));
    pool.remember(OriginFlag::H1Only, "example.com", 443, None);
    assert!(pool.recalls(OriginFlag::H1Only, "example.com", 443, None));
    assert!(!pool.recalls(
        OriginFlag::H1Only,
        "example.com",
        443,
        Some(&crate::net::ProxyUrl::parse("http://proxy:1").unwrap())
    ));
    assert!(!pool.recalls(OriginFlag::H1Only, "example.com", 8443, None));
}

#[test]
fn alpn_h1_memory_expires() {
    let pool = Pool::new();
    pool.remember(OriginFlag::H1Only, "example.com", 443, None);
    let key = ("example.com".to_string(), 443, None);
    let now = std::time::SystemTime::now();
    assert!(
        crate::util::lock(&pool.memory.origins)[super::OriginFlag::H1Only as usize]
            .contains(&key, now)
    );
    let later = now + super::OriginFlag::H1Only.ttl() + Duration::from_secs(1);
    assert!(
        !crate::util::lock(&pool.memory.origins)[super::OriginFlag::H1Only as usize]
            .contains(&key, later)
    );
}

#[test]
fn expiring_set_is_bounded() {
    use crate::util::expiring::{ExpiringSet, MAX_ENTRIES};
    let now = std::time::SystemTime::now();
    let mut set = ExpiringSet::default();
    for i in 0..=MAX_ENTRIES {
        set.insert(i, now + Duration::from_secs(i as u64 + 1), now);
    }
    assert!(!set.contains(&0, now));
    assert!(set.contains(&MAX_ENTRIES, now));
}

#[cfg(feature = "http3")]
#[test]
fn alt_svc_withdrawal_forgets_h3() {
    let advertised = "h3=\":443\"; ma=86400";
    for withdrawal in [
        "h3=\":443\"; ma=0",
        "clear",
        "h2=\":443\"",
        "h3=\":443\"; ma=60, clear",
    ] {
        let pool = Pool::new();
        pool.note_alt_svc("example.com", 443, &[advertised], Duration::ZERO);
        pool.note_alt_svc("example.com", 443, &[withdrawal], Duration::ZERO);
        assert!(!pool.knows_h3("example.com", 443), "{withdrawal}");
    }
}

#[cfg(feature = "http3")]
#[test]
fn alt_svc_expires_after_max_age() {
    let now = std::time::SystemTime::now();
    let mut cache = super::alt_svc::AltSvcCache::default();
    cache.note(
        "example.com",
        443,
        &["h3=\":443\"; ma=60"],
        Duration::ZERO,
        now,
    );
    assert!(cache.knows_h3("example.com", 443, now + Duration::from_secs(59)));
    assert!(!cache.knows_h3("example.com", 443, now + Duration::from_secs(61)));
}

#[cfg(feature = "http3")]
#[test]
fn alt_svc_ignores_separators_inside_quoted_parameters() {
    let pool = Pool::new();
    pool.note_alt_svc(
        "example.com",
        443,
        &["h3=\":443\"; ma=60; note=\"one, clear, two\""],
        Duration::ZERO,
    );
    assert!(pool.knows_h3("example.com", 443));
}

#[cfg(feature = "http3")]
#[test]
fn a_failed_http3_origin_is_skipped_until_cleared() {
    let pool = Pool::new();
    assert!(!pool.recalls(
        OriginFlag::H3Broken,
        "example.com",
        443,
        Some(&crate::net::ProxyUrl::parse("socks5://p:1").unwrap())
    ));
    pool.remember(
        OriginFlag::H3Broken,
        "example.com",
        443,
        Some(&crate::net::ProxyUrl::parse("socks5://p:1").unwrap()),
    );
    assert!(pool.recalls(
        OriginFlag::H3Broken,
        "example.com",
        443,
        Some(&crate::net::ProxyUrl::parse("socks5://p:1").unwrap())
    ));
    assert!(!pool.recalls(OriginFlag::H3Broken, "example.com", 443, None));
    pool.forget(
        super::OriginFlag::H3Broken,
        "example.com",
        443,
        Some(&crate::net::ProxyUrl::parse("socks5://p:1").unwrap()),
    );
    assert!(!pool.recalls(
        OriginFlag::H3Broken,
        "example.com",
        443,
        Some(&crate::net::ProxyUrl::parse("socks5://p:1").unwrap())
    ));
}

async fn h2_client() -> (crate::h2::client::H2Client, tokio::io::DuplexStream) {
    let (client, server) = tokio::io::duplex(64 * 1024);
    let config = crate::h2::config::H2Config::from_profile(&crate::Browser::default().profile().h2)
        .expect("profile");
    let handle = crate::h2::start(client, config).await.expect("handshake");
    (handle, server)
}

#[tokio::test]
async fn a_failure_on_a_replaced_connection_keeps_the_replacement() {
    let pool = Pool::new();
    let key = PoolKey {
        scheme: crate::wire::Scheme::Https,
        port: 443,
        ..h1_key("replaced.test")
    };
    let (old, _old_io) = h2_client().await;
    let (new, _new_io) = h2_client().await;
    pool.install_mux(key.clone(), old.clone(), TlsInfo::default());
    pool.invalidate(&key, &old);
    pool.install_mux(key.clone(), new.clone(), TlsInfo::default());

    pool.invalidate(&key, &old);

    let pooled = pool.checkout_mux::<crate::h2::client::H2Client>(&key);
    assert!(pooled.is_some_and(|(conn, _)| conn.same(&new)));
}
