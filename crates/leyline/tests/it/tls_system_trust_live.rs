#[cfg(all(target_os = "macos", feature = "http3"))]
#[tokio::test]
#[ignore = "live: needs UDP network access and current public PKI"]
async fn macos_system_trust_accepts_public_quic_chain() {
    let session = leyline::Session::builder()
        .browser(leyline::Browser::default())
        .protocol(leyline::Protocol::Http3)
        .tls(leyline::Tls::new().env_roots(false))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("build HTTP/3 session");
    let response = session
        .get("https://cloudflare.com")
        .send()
        .await
        .expect("public HTTP/3 request with system trust");
    assert_eq!(response.version(), http::Version::HTTP_3);
    assert!(response.tls_version().is_some());
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "live: needs network and current public PKI"]
async fn macos_system_trust_accepts_public_chain() {
    let session = leyline::Session::builder()
        .browser(leyline::Browser::default())
        .tls(leyline::Tls::new().env_roots(false))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("build system-trust session");
    let result = session.get("https://httpbin.org/get").send().await;

    let response = result.expect("macOS system trust accepts a public chain");
    assert_ne!(response.version(), http::Version::HTTP_3);
    assert!(response.tls_version().is_some());
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "live: needs LEYLINE_TEST_PROXY and current public PKI"]
async fn macos_system_trust_accepts_public_chain_through_proxy() {
    let proxy = std::env::var("LEYLINE_TEST_PROXY").expect("LEYLINE_TEST_PROXY is required");
    let session = leyline::Session::builder()
        .browser(leyline::Browser::Chrome150)
        .platform(leyline::Platform::Windows)
        .proxy(proxy.as_str())
        .build()
        .expect("build production-shaped session");
    let result = session
        .request(http::Method::GET, "https://httpbin.org/get")
        .send()
        .await;

    assert!(
        result.is_ok(),
        "macOS system trust rejected the public chain through proxy: {:?}",
        result.err()
    );
}
