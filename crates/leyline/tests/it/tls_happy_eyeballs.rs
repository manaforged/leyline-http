use std::time::Duration;

use leyline::tls::HappyEyeballsConfig;
use leyline::{Browser, Session, TlsError, TlsTrustConfig};

fn eyeballs(resolve_delay: Duration, attempt_limit: usize) -> HappyEyeballsConfig {
    HappyEyeballsConfig::new()
        .resolve_delay(resolve_delay)
        .attempt_limit(attempt_limit)
}

#[tokio::test]
async fn default_happy_eyeballs_is_250ms() {
    assert_eq!(
        HappyEyeballsConfig::default(),
        eyeballs(Duration::from_millis(250), 8)
    );
}

#[test]
fn invalid_der_root_is_rejected_at_build_time() {
    let err = Session::builder()
        .browser(Browser::Chrome146)
        .tls_trust(TlsTrustConfig::new().add_ca_der([1, 2, 3, 4]))
        .build()
        .expect_err("invalid DER CA should fail TLS setup");
    assert!(matches!(err.tls(), Some(TlsError::SslConfig(_))));
}
