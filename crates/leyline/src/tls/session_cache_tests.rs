use super::SessionCache;
use crate::util::lock;
use leyline_bssl::ssl::{SslContextBuilder, SslMethod};

#[test]
fn the_session_cache_keeps_at_most_256_entries() {
    let mut context = SslContextBuilder::new(SslMethod::tls()).expect("context");
    let tls = crate::Tls::new().system_roots(false).env_roots(false);
    let identity = crate::tls::trust::wire_configured_trust(&mut context, &tls).expect("trust");
    let cache = SessionCache::new(identity);

    for n in 0..300 {
        lock(&cache.entries).put(format!("host-{n}:443|"), vec![0; 8]);
    }

    let entries = lock(&cache.entries);
    assert_eq!(entries.len(), 256);
    assert!(!entries.contains("host-0:443|"));
    assert!(entries.contains("host-299:443|"));
}
