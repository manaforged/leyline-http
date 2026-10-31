use super::*;
use crate::profile::Platform;

fn plain(profile: &BrowserProfile, trust: &Tls) -> FingerprintConnector {
    FingerprintConnector::new_with_trust(profile, Platform::Linux.tcp_profile(), trust)
        .expect("connector build")
}

fn same_context(a: &FingerprintConnector, b: &FingerprintConnector) -> bool {
    std::ptr::eq(a.ssl_connector.context(), b.ssl_connector.context())
}

#[test]
fn plain_sessions_share_one_tls_context_per_trust() {
    let bare = BrowserProfile::bare_shared();
    let trust = Tls::new().env_roots(false).system_roots(false);
    let pinned = trust.clone().pin_sha256([7; 32]);

    let first = plain(&bare, &trust);
    let second = plain(&bare, &trust);
    let other_trust = plain(&bare, &pinned);
    let unshared_profile = plain(&BrowserProfile::bare(), &trust);

    assert!(same_context(&first, &second));
    assert!(!same_context(&first, &other_trust));
    assert!(!same_context(&first, &unshared_profile));
}
