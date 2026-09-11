#[cfg(all(target_os = "macos", feature = "http3"))]
#[tokio::test]
#[ignore = "live: needs UDP network access and current public PKI"]
async fn macos_system_trust_accepts_public_quic_chain() {
    let session = leyline::Session::builder()
        .chrome()
        .http3()
        .without_env_roots()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("build HTTP/3 session");
    let response = session
        .get("https://cloudflare.com")
        .send()
        .await
        .expect("public HTTP/3 request with system trust");
    assert_eq!(response.version(), leyline::HttpVersion::Http3);
    assert!(response.tls_version().is_some());
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "live: needs network and current public PKI"]
async fn macos_system_trust_accepts_example_chain() {
    let session = leyline::Session::builder()
        .chrome()
        .without_env_roots()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("build system-trust session");
    let result = session
        .get(
            "https://store.example.com/",
        )
        .await;

    let response = result.expect("macOS system trust accepts the test site's public chain");
    assert_ne!(response.version(), leyline::HttpVersion::Http3);
    assert!(response.tls_version().is_some());
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "live: needs LEYLINE_TEST_PROXY and current public PKI"]
async fn macos_system_trust_accepts_example_chain_through_proxy() {
    let proxy = std::env::var("LEYLINE_TEST_PROXY").expect("LEYLINE_TEST_PROXY is required");
    let session = leyline::Session::builder()
        .browser(leyline::Browser::Chrome150)
        .platform(leyline::Platform::Windows)
        .proxy(&proxy)
        .build()
        .expect("build production-shaped session");
    let result = session
        .request(http::Method::GET,
            "https://store.example.com/",
        )
        .send()
        .await;

    assert!(
        result.is_ok(),
        "macOS system trust rejected the test site through proxy: {:?}",
        result.err()
    );
}
