use super::{OriginFlag, Pool};
use crate::pool::types::{PoolKey, PooledConn, TlsInfo, Transport};
use std::time::{Duration, Instant};

fn h1_key(host: &str) -> PoolKey {
    PoolKey {
        scheme: "http".into(),
        host: host.to_string(),
        port: 80,
        proxy: None,
        transport: Transport::Tcp,
        partition: 0,
        lane: 0,
    }
}

fn insert_empty_h1(pool: &Pool, host: &str, last_use: Instant) {
    crate::util::write(&pool.inner).insert(
        h1_key(host),
        PooledConn::H1 {
            idle: crate::pool::types::H1Idle::default(),
            last_use,
            tls: TlsInfo::default(),
        },
    );
}

#[test]
fn evict_idle_drops_abandoned_empty_h1_entry() {
    let pool = Pool::with_limits(Duration::from_millis(20), 2048, 6);
    let stale = Instant::now()
        .checked_sub(Duration::from_millis(40))
        .expect("monotonic clock is >40ms past its epoch");
    insert_empty_h1(&pool, "abandoned", stale);

    pool.evict_idle(std::time::Instant::now());

    assert_eq!(
        pool.stats().entries,
        0,
        "an empty H1 entry idle past the timeout must be reaped, not linger until LRU"
    );
}

#[test]
fn evict_idle_keeps_empty_h1_entry_with_live_checkouts() {
    let pool = Pool::with_limits(Duration::from_secs(300), 2048, 6);
    insert_empty_h1(&pool, "in-flight", Instant::now());

    pool.evict_idle(std::time::Instant::now());

    assert_eq!(
        pool.stats().entries,
        1,
        "an empty H1 entry touched within the idle window (checkouts in flight) must survive"
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
        Some(&crate::core::ProxyUrl::parse("http://proxy:1").unwrap())
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
        crate::util::lock(&pool.origin_memory)[super::OriginFlag::H1Only as usize]
            .contains(&key, now)
    );
    let later = now + super::OriginFlag::H1Only.ttl() + Duration::from_secs(1);
    assert!(
        !crate::util::lock(&pool.origin_memory)[super::OriginFlag::H1Only as usize]
            .contains(&key, later)
    );
}

#[test]
fn expiring_set_is_bounded() {
    use super::expiring::{ExpiringSet, MAX_ENTRIES};
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
        Some(&crate::core::ProxyUrl::parse("socks5://p:1").unwrap())
    ));
    pool.remember(
        OriginFlag::H3Broken,
        "example.com",
        443,
        Some(&crate::core::ProxyUrl::parse("socks5://p:1").unwrap()),
    );
    assert!(pool.recalls(
        OriginFlag::H3Broken,
        "example.com",
        443,
        Some(&crate::core::ProxyUrl::parse("socks5://p:1").unwrap())
    ));
    assert!(!pool.recalls(OriginFlag::H3Broken, "example.com", 443, None));
    pool.forget(
        super::OriginFlag::H3Broken,
        "example.com",
        443,
        Some(&crate::core::ProxyUrl::parse("socks5://p:1").unwrap()),
    );
    assert!(!pool.recalls(
        OriginFlag::H3Broken,
        "example.com",
        443,
        Some(&crate::core::ProxyUrl::parse("socks5://p:1").unwrap())
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
        scheme: "https".into(),
        port: 443,
        ..h1_key("replaced.test")
    };
    let (old, _old_io) = h2_client().await;
    let (new, _new_io) = h2_client().await;
    pool.install_mux(key.clone(), old.clone(), TlsInfo::default());
    crate::util::write(&pool.inner).remove(&key);
    pool.install_mux(key.clone(), new.clone(), TlsInfo::default());

    pool.invalidate(&key, &old);

    let pooled = pool.checkout_mux::<crate::h2::client::H2Client>(&key);
    assert!(pooled.is_some_and(|(conn, _)| conn.same(&new)));
}
