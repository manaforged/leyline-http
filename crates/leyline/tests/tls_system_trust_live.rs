#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "live: needs network and current public PKI"]
async fn macos_system_trust_accepts_example_chain() {
    let result = leyline::Session::chrome()
        .get(
            "https://store.example.com/",
        )
        .await;

    assert!(
        result.is_ok(),
        "macOS system trust rejected the test site's public chain: {:?}",
        result.err()
    );
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
