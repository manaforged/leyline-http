use leyline::{Browser, Session, Tls, TlsError};

#[test]
fn invalid_der_root_is_rejected_at_build_time() {
    let err = Session::builder()
        .browser(Browser::Chrome146)
        .tls(Tls::new().ca_der([1, 2, 3, 4]))
        .build()
        .expect_err("invalid DER CA should fail TLS setup");
    assert!(matches!(
        std::error::Error::source(&err).and_then(|s| s.downcast_ref::<leyline::TlsError>()),
        Some(TlsError::SslConfig(_))
    ));
}
