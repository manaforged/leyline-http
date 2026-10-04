#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
#![expect(
    clippy::panic,
    reason = "test harness helper: explicit panic on unexpected error shape is the assertion"
)]
use leyline::{Browser, Platform};
use serde_json::Value;

const PEET_URL: &str = "https://tls.peet.ws/api/all";

#[test]
fn every_browser_variant_has_profile() {
    let reg = leyline::profile::ProfileRegistry::builtin();
    for browser in Browser::all().iter().copied() {
        assert!(
            reg.get_browser(browser).is_some(),
            "no profile for {browser}"
        );
    }
}

#[test]
fn every_profile_has_fingerprint_expectation() {
    let reg = leyline::profile::ProfileRegistry::builtin();
    for browser in Browser::all().iter().copied() {
        let profile = reg.get_browser(browser).unwrap();
        let has_ja4 = profile.expected_ja4().is_some();
        let has_h2 = profile.expected_h2_fingerprint().is_some();
        assert!(
            has_ja4 || has_h2,
            "{browser} has no expected fingerprints in TOML"
        );
    }
}

#[test]
fn h2_fingerprints_match_toml_expectations() {
    let reg = leyline::profile::ProfileRegistry::builtin();
    let mut checked = 0;
    for browser in Browser::all().iter().copied() {
        let profile = reg.get_browser(browser).unwrap();
        if let Some(expected) = profile.expected_h2_fingerprint() {
            let h2 = leyline::h2::H2Config::from_profile(&profile.h2).unwrap();
            let actual = h2.akamai_fingerprint();
            assert_eq!(actual, expected, "H2 mismatch for {browser}");
            checked += 1;
        }
    }
    assert!(
        checked >= 10,
        "expected all 10 H2 fingerprints to be checked, got {checked}"
    );
}

#[test]
fn h2_per_platform_overrides_resolve() {
    let reg = leyline::profile::ProfileRegistry::builtin();
    for browser in [Browser::Chrome145, Browser::Chrome146, Browser::Chrome147] {
        let profile = reg.get_browser(browser).unwrap();
        let resolved = profile.h2.resolve_for_platform(Platform::MacOS).unwrap();
        let h2 = leyline::h2::H2Config::from_profile(&resolved).unwrap();
        let actual = h2.akamai_fingerprint();
        let expected = profile
            .h2
            .platforms
            .get("macos")
            .and_then(|over| over.fingerprint.as_ref())
            .and_then(|fp| fp.akamai.as_deref())
            .or_else(|| profile.expected_h2_fingerprint())
            .unwrap_or_else(|| panic!("{browser} has no macos H2 fingerprint expectation"));
        assert_eq!(actual, expected, "{browser} macOS H2 fingerprint mismatch");
        assert!(
            !actual.contains(";8:1"),
            "{browser} macOS Akamai must omit setting 8: {actual}"
        );
    }
}

#[test]
fn session_builder_resolves_all_valid_combos() {
    let combos: Vec<(Browser, Platform)> = vec![
        (Browser::Chrome147, Platform::Windows),
        (Browser::Chrome147, Platform::MacOS),
        (Browser::Chrome147, Platform::Linux),
        (Browser::Chrome147, Platform::Android),
        (Browser::Chrome148, Platform::Windows),
        (Browser::Chrome148, Platform::MacOS),
        (Browser::Chrome148, Platform::Linux),
        (Browser::Chrome148, Platform::Android),
        (Browser::Chrome149, Platform::Windows),
        (Browser::Chrome149, Platform::MacOS),
        (Browser::Chrome149, Platform::Linux),
        (Browser::Chrome149, Platform::Android),
        (Browser::Chrome150, Platform::Windows),
        (Browser::Chrome150, Platform::MacOS),
        (Browser::Chrome150, Platform::Linux),
        (Browser::Chrome150, Platform::Android),
        (Browser::Chrome146, Platform::Windows),
        (Browser::Chrome146, Platform::Android),
        (Browser::Chrome145, Platform::Windows),
        (Browser::Chrome145, Platform::Android),
        (Browser::Brave146, Platform::MacOS),
        (Browser::Firefox148, Platform::Windows),
        (Browser::Firefox148, Platform::Linux),
        (Browser::Firefox148, Platform::Android),
        (Browser::Firefox150, Platform::Windows),
        (Browser::Firefox150, Platform::Linux),
        (Browser::Firefox150, Platform::Android),
        (Browser::Firefox151, Platform::Windows),
        (Browser::Firefox151, Platform::Linux),
        (Browser::Firefox151, Platform::Android),
        (Browser::Firefox152, Platform::Windows),
        (Browser::Firefox152, Platform::Linux),
        (Browser::Firefox152, Platform::Android),
        (Browser::Safari18, Platform::MacOS),
        (Browser::OkHttpAndroid10, Platform::Android),
        (Browser::SafariIOS17, Platform::IOS),
        (Browser::SafariIOS18, Platform::IOS),
        (Browser::CfnetworkIOS18, Platform::IOS),
        (Browser::CfnetworkMacOS26, Platform::MacOS),
    ];
    for (browser, platform) in combos {
        let result = leyline::Session::builder()
            .browser(browser)
            .platform(platform)
            .build();
        assert!(result.is_ok(), "failed to build {browser} on {platform}");
    }
}

#[test]
fn crate_root_context_helpers_work() {
    let tls = leyline::TlsContext::from_profile(
        Browser::Chrome147.profile(),
        leyline::TlsMinVersion::Tls12,
    );
    assert!(tls.is_ok(), "tls context failed: {:?}", tls.err());

    #[cfg(feature = "http3")]
    {
        let quic = leyline::TlsContext::from_profile(
            Browser::Chrome147.profile(),
            leyline::TlsMinVersion::Tls13,
        );
        assert!(quic.is_ok(), "quic context failed: {:?}", quic.err());
    }

    for browser in Browser::all().iter().copied() {
        assert!(
            leyline::TlsContext::from_profile(browser.profile(), leyline::TlsMinVersion::Tls12)
                .is_ok(),
            "tls context failed for {browser}"
        );
    }
}

#[test]
fn profile_helper_returns_builtin() {
    let p = Browser::Chrome147.profile();
    assert_eq!(p.meta.browser, "chrome");
    assert_eq!(p.meta.version, 147);
    assert!(!p.tls.ciphers.is_empty());
}

#[test]
fn session_builder_rejects_http3_with_proxy_at_build_time() {
    let err = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .protocol(leyline::ProtocolPolicy::Http3)
        .proxy("http://127.0.0.1:8080")
        .build()
        .expect_err("should reject http3+proxy at build");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("http/3") || msg.contains("http3") || msg.contains("proxy"),
        "error message should mention http3/proxy: {msg}"
    );
}

#[cfg(feature = "http3")]
#[tokio::test]
async fn http3_session_rejects_per_request_proxy_at_send_time() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .protocol(leyline::ProtocolPolicy::Http3)
        .build()
        .expect("http3 session without proxy builds");
    let err = session
        .request(http::Method::GET, "https://example.com/")
        .proxy("http://127.0.0.1:9")
        .send()
        .await
        .expect_err("h3 + per-request proxy must refuse, not dial direct");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("http/3") || msg.contains("http3") || msg.contains("prox"),
        "error should mention http3/proxy, got: {msg}"
    );
}

#[cfg(feature = "http3")]
#[tokio::test]
async fn race_proxy() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .protocol(leyline::ProtocolPolicy::Race)
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .expect("race session builds");
    let result = session
        .request(http::Method::GET, "https://example.com/")
        .proxy("http://127.0.0.1:9")
        .send()
        .await;
    assert!(
        result.is_err(),
        "request through a dead proxy must fail; success means the H3 race \
         leg bypassed the proxy and dialed direct"
    );
}

#[tokio::test]
async fn request_builder_timeout_overrides_session_default() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap();
    let start = std::time::Instant::now();
    let err = session
        .request(http::Method::GET, "https://192.0.2.1/")
        .timeout(std::time::Duration::from_millis(50))
        .send()
        .await
        .expect_err("should time out on unroutable address");
    let elapsed = start.elapsed();
    assert!(
        err.is_timeout(),
        "expected Error::new(Kind::Timeout), got {err:?}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "per-request timeout didn't fire — elapsed {elapsed:?}"
    );
}

#[test]
fn session_shortcuts_work() {
    assert_eq!(
        format!("{}", leyline::Session::browser(Browser::default())),
        format!(
            "Session({}, {}, proxy=none)",
            Browser::default(),
            Platform::Windows
        )
    );
    let firefox = leyline::Session::builder()
        .browser(Browser::latest(leyline::Family::Firefox))
        .platform(Platform::Windows)
        .build()
        .unwrap();
    assert_eq!(
        format!("{firefox}"),
        format!(
            "Session({}, {}, proxy=none)",
            Browser::latest(leyline::Family::Firefox),
            Platform::Windows
        )
    );
    let safari = leyline::Session::builder()
        .browser(Browser::Safari26)
        .platform(Platform::MacOS)
        .build()
        .unwrap();
    assert_eq!(
        format!("{safari}"),
        format!(
            "Session({}, {}, proxy=none)",
            Browser::Safari26,
            Platform::MacOS
        )
    );
}

#[tokio::test]
async fn offline_http_connect_proxy_wire_bytes() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = Vec::with_capacity(512);
        let mut tmp = [0u8; 256];
        loop {
            let n = stream.read(&mut tmp).await.unwrap();
            if n == 0 {
                return Err("client closed before request headers".to_string());
            }
            buf.extend_from_slice(&tmp[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let req = String::from_utf8_lossy(&buf).into_owned();

        if !req.starts_with("CONNECT target.example.com:443 HTTP/1.1\r\n") {
            return Err(format!("bad request line: {req:?}"));
        }
        if !req.contains("Host: target.example.com:443") {
            return Err(format!("missing Host header: {req:?}"));
        }
        if !req.contains("Proxy-Authorization: Basic ") {
            return Err(format!("missing Proxy-Authorization: {req:?}"));
        }

        stream
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .await
            .unwrap();
        let _ = stream.shutdown().await;
        Ok(req)
    });

    let proxy_url = format!(
        "http://testuser:testpass@{}:{}",
        proxy_addr.ip(),
        proxy_addr.port()
    );
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .proxy(&proxy_url)
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();

    let _ = session.get("https://target.example.com/").await;

    let recv = server.await.unwrap();
    match recv {
        Ok(req) => println!(
            "✓ offline HTTP CONNECT proxy wire bytes OK ({} bytes header)",
            req.len()
        ),
        Err(e) => panic!("mock proxy rejected CONNECT request: {e}"),
    }
}

#[cfg(feature = "socks")]
#[tokio::test]
async fn offline_socks5_proxy_wire_bytes() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        let mut hdr = [0u8; 2];
        stream.read_exact(&mut hdr).await.unwrap();
        assert_eq!(hdr[0], 0x05, "socks5 version wrong");
        let n_methods = hdr[1] as usize;
        let mut methods = vec![0u8; n_methods];
        stream.read_exact(&mut methods).await.unwrap();
        assert!(
            methods.contains(&0x02),
            "expected USERNAME/PASSWORD (0x02) in offered auth methods"
        );

        stream.write_all(&[0x05, 0x02]).await.unwrap();

        let mut sub = [0u8; 2];
        stream.read_exact(&mut sub).await.unwrap();
        assert_eq!(sub[0], 0x01, "socks5 auth subneg version wrong");
        let ulen = sub[1] as usize;
        let mut uname = vec![0u8; ulen];
        stream.read_exact(&mut uname).await.unwrap();
        let mut plen_buf = [0u8; 1];
        stream.read_exact(&mut plen_buf).await.unwrap();
        let mut pass = vec![0u8; plen_buf[0] as usize];
        stream.read_exact(&mut pass).await.unwrap();
        assert_eq!(uname, b"testuser");
        assert_eq!(pass, b"testpass");

        stream.write_all(&[0x01, 0x00]).await.unwrap();

        let mut conn_hdr = [0u8; 4];
        stream.read_exact(&mut conn_hdr).await.unwrap();
        assert_eq!(conn_hdr[0], 0x05);
        assert_eq!(conn_hdr[1], 0x01, "expected CONNECT cmd");
        assert_eq!(conn_hdr[3], 0x03, "expected domain ATYP");
        let mut dlen = [0u8; 1];
        stream.read_exact(&mut dlen).await.unwrap();
        let mut dom = vec![0u8; dlen[0] as usize];
        stream.read_exact(&mut dom).await.unwrap();
        let mut port = [0u8; 2];
        stream.read_exact(&mut port).await.unwrap();
        assert_eq!(dom, b"target.example.com");
        assert_eq!(u16::from_be_bytes(port), 443);

        stream
            .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await
            .unwrap();
        let _ = stream.shutdown().await;
    });

    let proxy_url = format!(
        "socks5://testuser:testpass@{}:{}",
        proxy_addr.ip(),
        proxy_addr.port()
    );
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .proxy(&proxy_url)
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();

    let _ = session.get("https://target.example.com/").await;

    server.await.unwrap();
    println!("✓ offline SOCKS5 wire bytes OK (RFC 1928 greet + auth + CONNECT)");
}

async fn peet(session: &leyline::Session) -> Value {
    let mut last_err = String::new();
    for attempt in 0..4 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
        }
        let resp = match session.get(PEET_URL).await {
            Ok(r) => r,
            Err(e) => {
                last_err = format!("request error: {e}");
                continue;
            }
        };
        if resp.status() != 200 {
            last_err = format!("status {}", resp.status());
            continue;
        }
        match serde_json::from_str(&resp.text().await.unwrap()) {
            Ok(v) => return v,
            Err(e) => last_err = format!("non-JSON: {e}"),
        }
    }
    panic!("tls.peet.ws failed after retries: {last_err}");
}

fn live_platform_for(browser: Browser) -> Platform {
    [
        Platform::Windows,
        Platform::MacOS,
        Platform::Linux,
        Platform::Android,
        Platform::IOS,
    ]
    .into_iter()
    .find(|&platform| {
        browser.for_platform(platform) == browser && browser.identity(platform, None).is_some()
    })
    .unwrap_or_else(|| panic!("{browser} defines no platform identity"))
}

fn normalize_akamai(fp: &str) -> String {
    fp.replace(";:1", ";8:1")
}

fn extract_sent_headers(peet_json: &Value) -> Vec<(String, String)> {
    let frames = peet_json["http2"]["sent_frames"]
        .as_array()
        .expect("no http2.sent_frames");
    for frame in frames {
        if frame["frame_type"].as_str() != Some("HEADERS") {
            continue;
        }
        let headers = match frame["headers"].as_array() {
            Some(h) => h,
            None => continue,
        };
        return headers
            .iter()
            .filter_map(|h| {
                let s = h.as_str()?;
                let (k, v) = s.split_once(": ")?;
                Some((k.to_string(), v.to_string()))
            })
            .collect();
    }
    panic!("no HEADERS frame in sent_frames");
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn denormalize_peet_quotes(raw: &str) -> String {
    if !raw.contains('\\') {
        return raw.to_string();
    }
    let mut s = raw.replace("\\\"", "\"");
    if s.ends_with('\\') {
        s.pop();
        s.push('"');
    }
    s
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_wire_audit_every_profile() {
    let reg = leyline::profile::ProfileRegistry::builtin();
    let mut checked = 0;

    for browser in Browser::all().iter().copied() {
        let platform = live_platform_for(browser);
        let session = leyline::Session::builder()
            .browser(browser)
            .platform(platform)
            .build()
            .unwrap();
        let json = peet(&session).await;
        let profile = reg.get_browser(browser).unwrap();

        let wire_ja4 = json["tls"]["ja4"].as_str().unwrap_or("?");
        let wire_ja3 = json["tls"]["ja3_hash"].as_str().unwrap_or("?");
        let wire_peet = json["tls"]["peetprint_hash"].as_str().unwrap_or("?");
        let wire_akamai =
            normalize_akamai(json["http2"]["akamai_fingerprint"].as_str().unwrap_or("?"));

        println!("\n## {browser} ({platform:?})");
        println!("  wire JA4       = {wire_ja4}");
        println!("  wire JA3 hash  = {wire_ja3}");
        println!("  wire peetprint = {wire_peet}");
        println!("  wire Akamai-H2 = {wire_akamai}");

        if let Some(exp) = profile.expected_ja4() {
            assert_eq!(wire_ja4, exp, "{browser} JA4 wire != golden");
        }
        if let Some(exp) = profile.expected_h2_fingerprint() {
            assert_eq!(
                wire_akamai,
                normalize_akamai(exp),
                "{browser} Akamai wire != golden"
            );
        }
        checked += 1;
    }
    assert!(checked >= 9, "expected >=9 profiles audited, got {checked}");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_chrome150_macos_matches_capture() {
    const SEC_CH_UA: &str = r#""Not;A=Brand";v="8", "Chromium";v="150", "Google Chrome";v="150""#;
    const JA4: &str = "t13d1516h2_8daaf6152771_806a8c22fdea";
    const H2: &str = "1:65536;2:0;4:6291456;6:262144|15663105|0|m,a,s,p";

    let session = leyline::Session::builder()
        .browser(Browser::Chrome150)
        .platform(Platform::MacOS)
        .build()
        .unwrap();
    let json = peet(&session).await;
    let headers = extract_sent_headers(&json);
    let sec_ch_ua = header_value(&headers, "sec-ch-ua").expect("no sec-ch-ua");
    let observed_h2 = json["http2"]["akamai_fingerprint"]
        .as_str()
        .expect("no http2.akamai_fingerprint");

    assert_eq!(denormalize_peet_quotes(sec_ch_ua), SEC_CH_UA);
    assert_eq!(json["tls"]["ja4"].as_str().expect("no tls.ja4"), JA4);
    assert_eq!(normalize_akamai(observed_h2), H2);
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_ja4_exact_match_chrome149() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome149)
        .build()
        .unwrap();
    let json = peet(&session).await;
    let ja4 = json["tls"]["ja4"].as_str().expect("no tls.ja4");

    let reg = leyline::profile::ProfileRegistry::builtin();
    let profile = reg.get_browser(Browser::Chrome149).unwrap();
    let expected = profile.expected_ja4().expect("Chrome 149 TOML missing JA4");

    assert_eq!(ja4, expected, "Chrome 149 JA4 mismatch");
    println!("✓ Chrome 149 JA4 exact match: {ja4}");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_ja4_exact_match_chrome148() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome148)
        .build()
        .unwrap();
    let json = peet(&session).await;
    let ja4 = json["tls"]["ja4"].as_str().expect("no tls.ja4");

    let reg = leyline::profile::ProfileRegistry::builtin();
    let profile = reg.get_browser(Browser::Chrome148).unwrap();
    let expected = profile.expected_ja4().expect("Chrome 148 TOML missing JA4");

    assert_eq!(ja4, expected, "Chrome 148 JA4 mismatch");
    println!("✓ Chrome 148 JA4 exact match: {ja4}");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_ja4_exact_match_chrome147() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .build()
        .unwrap();
    let json = peet(&session).await;
    let ja4 = json["tls"]["ja4"].as_str().expect("no tls.ja4");

    let reg = leyline::profile::ProfileRegistry::builtin();
    let profile = reg.get_browser(Browser::Chrome147).unwrap();
    let expected = profile.expected_ja4().expect("Chrome 147 TOML missing JA4");

    assert_eq!(ja4, expected, "Chrome 147 JA4 mismatch");
    println!("✓ Chrome 147 JA4 exact match: {ja4}");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_ja4_exact_match_chrome146() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome146)
        .build()
        .unwrap();
    let json = peet(&session).await;
    let ja4 = json["tls"]["ja4"].as_str().expect("no tls.ja4");

    let reg = leyline::profile::ProfileRegistry::builtin();
    let profile = reg.get_browser(Browser::Chrome146).unwrap();
    let expected = profile.expected_ja4().expect("Chrome 146 TOML missing JA4");

    assert_eq!(ja4, expected, "Chrome 146 JA4 mismatch");
    println!("✓ Chrome 146 JA4 exact match: {ja4}");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_ja4_exact_match_chrome145() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome145)
        .build()
        .unwrap();
    let json = peet(&session).await;
    let ja4 = json["tls"]["ja4"].as_str().expect("no tls.ja4");

    let reg = leyline::profile::ProfileRegistry::builtin();
    let profile = reg.get_browser(Browser::Chrome145).unwrap();
    let expected = profile.expected_ja4().expect("Chrome 145 TOML missing JA4");

    assert_eq!(ja4, expected, "Chrome 145 JA4 mismatch");
    println!("✓ Chrome 145 JA4 exact match: {ja4}");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_ja4_exact_match_firefox150() {
    let session = leyline::Session::builder()
        .browser(Browser::Firefox150)
        .build()
        .unwrap();
    let json = peet(&session).await;
    let ja4 = json["tls"]["ja4"].as_str().expect("no tls.ja4");

    let reg = leyline::profile::ProfileRegistry::builtin();
    let profile = reg.get_browser(Browser::Firefox150).unwrap();
    let expected = profile
        .expected_ja4()
        .expect("Firefox 150 TOML missing JA4");

    assert_eq!(ja4, expected, "Firefox 150 JA4 mismatch");
    println!("✓ Firefox 150 JA4 exact match: {ja4}");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_ja4_exact_match_firefox151() {
    let session = leyline::Session::builder()
        .browser(Browser::Firefox151)
        .build()
        .unwrap();
    let json = peet(&session).await;
    let ja4 = json["tls"]["ja4"].as_str().expect("no tls.ja4");

    let reg = leyline::profile::ProfileRegistry::builtin();
    let profile = reg.get_browser(Browser::Firefox151).unwrap();
    let expected = profile
        .expected_ja4()
        .expect("Firefox 151 TOML missing JA4");

    assert_eq!(ja4, expected, "Firefox 151 JA4 mismatch");
    println!("✓ Firefox 151 JA4 exact match: {ja4}");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_ja4_exact_match_firefox152() {
    let session = leyline::Session::builder()
        .browser(Browser::Firefox152)
        .build()
        .unwrap();
    let json = peet(&session).await;
    let ja4 = json["tls"]["ja4"].as_str().expect("no tls.ja4");

    let reg = leyline::profile::ProfileRegistry::builtin();
    let profile = reg.get_browser(Browser::Firefox152).unwrap();
    let expected = profile
        .expected_ja4()
        .expect("Firefox 152 TOML missing JA4");

    assert_eq!(ja4, expected, "Firefox 152 JA4 mismatch");
    println!("✓ Firefox 152 JA4 exact match: {ja4}");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_ja4_exact_match_safari18() {
    let session = leyline::Session::builder()
        .browser(Browser::Safari18)
        .platform(live_platform_for(Browser::Safari18))
        .build()
        .unwrap();
    let json = peet(&session).await;
    let ja4 = json["tls"]["ja4"].as_str().expect("no tls.ja4");

    let reg = leyline::profile::ProfileRegistry::builtin();
    let profile = reg.get_browser(Browser::Safari18).unwrap();
    let expected = profile.expected_ja4().expect("Safari 18 TOML missing JA4");

    assert_eq!(ja4, expected, "Safari 18 JA4 mismatch");
    println!("✓ Safari 18 JA4 exact match: {ja4}");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_ja4_exact_match_every_profile_with_expectation() {
    let reg = leyline::profile::ProfileRegistry::builtin();
    let mut checked = 0;
    let mut skipped: Vec<String> = Vec::new();

    for browser in Browser::all().iter().copied() {
        let profile = reg.get_browser(browser).unwrap();
        let expected = match profile.expected_ja4() {
            Some(e) => e,
            None => {
                skipped.push(format!("{browser} (no JA4 in TOML)"));
                continue;
            }
        };

        let platform = live_platform_for(browser);
        let session = leyline::Session::builder()
            .browser(browser)
            .platform(platform)
            .build()
            .unwrap();
        let json = peet(&session).await;
        let ja4 = json["tls"]["ja4"].as_str().expect("no tls.ja4");
        assert_eq!(ja4, expected, "{browser} JA4 mismatch");
        println!("✓ {browser:30} JA4: {ja4}");
        checked += 1;
    }

    for s in &skipped {
        println!("- {s}");
    }
    assert!(
        checked >= 9,
        "expected at least 9 profiles to have JA4 asserted, got {checked}"
    );
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h2_akamai_every_profile() {
    let reg = leyline::profile::ProfileRegistry::builtin();
    let mut checked = 0;

    for browser in Browser::all().iter().copied() {
        let platform = live_platform_for(browser);

        let session = leyline::Session::builder()
            .browser(browser)
            .platform(platform)
            .build()
            .unwrap();

        let json = peet(&session).await;
        let observed = json["http2"]["akamai_fingerprint"]
            .as_str()
            .expect("no http2.akamai_fingerprint");
        let observed = normalize_akamai(observed);

        let profile = reg.get_browser(browser).unwrap();
        let expected = profile
            .expected_h2_fingerprint()
            .unwrap_or_else(|| panic!("{browser} TOML missing H2 fingerprint"));

        assert_eq!(observed, expected, "H2 Akamai mismatch for {browser}");
        println!("✓ {browser:30} H2: {observed}");
        checked += 1;
    }

    assert_eq!(
        checked,
        Browser::all().len(),
        "expected to check every first-class profile"
    );
}

async fn observe_ttl(browser: Browser, platform: Platform) -> i64 {
    let session = leyline::Session::builder()
        .browser(browser)
        .platform(platform)
        .build()
        .unwrap();
    let json = peet(&session).await;
    json["tcpip"]["ip"]["ttl"]
        .as_i64()
        .expect("no tcpip.ip.ttl")
}

fn initial_ttl(observed: i64) -> i64 {
    [64, 128, 255]
        .into_iter()
        .find(|&t| observed <= t)
        .unwrap_or(255)
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_tcp_windows_ttl_is_128() {
    let ttl = observe_ttl(Browser::Chrome147, Platform::Windows).await;
    assert_eq!(
        initial_ttl(ttl),
        128,
        "Windows initial TTL should be 128, observed {ttl} (rounded to {})",
        initial_ttl(ttl)
    );
    println!("✓ Windows TCP TTL: observed {ttl} (initial 128)");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_tcp_linux_ttl_is_64() {
    let ttl = observe_ttl(Browser::Chrome147, Platform::Linux).await;
    assert_eq!(
        initial_ttl(ttl),
        64,
        "Linux initial TTL should be 64, observed {ttl} (rounded to {})",
        initial_ttl(ttl)
    );
    println!("✓ Linux TCP TTL: observed {ttl} (initial 64)");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_tcp_windows_distinguishable_from_linux() {
    let win_ttl = observe_ttl(Browser::Chrome147, Platform::Windows).await;
    let linux_ttl = observe_ttl(Browser::Chrome147, Platform::Linux).await;
    assert!(
        win_ttl > linux_ttl + 40,
        "Windows TTL ({win_ttl}) not sufficiently greater than Linux TTL ({linux_ttl}) — \
         TCP profile may not be applied"
    );
    println!("✓ TCP TTL distinguishes Windows ({win_ttl}) from Linux ({linux_ttl})");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_chrome147_ciphers_match_profile_order() {
    let session = leyline::Session::browser(Browser::default());
    let json = peet(&session).await;

    let reg = leyline::profile::ProfileRegistry::builtin();
    let profile = reg.get_browser(Browser::Chrome147).unwrap();
    let expected_ciphers: Vec<&str> = profile.tls.ciphers.iter().map(|s| s.as_str()).collect();

    let observed: Vec<String> = json["tls"]["ciphers"]
        .as_array()
        .expect("no tls.ciphers")
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();

    let observed_no_grease: Vec<&str> = observed
        .iter()
        .filter(|c| !c.contains("GREASE") && !c.contains("Unknown"))
        .map(|s| s.as_str())
        .collect();

    assert_eq!(
        observed_no_grease, expected_ciphers,
        "Chrome 147 cipher list order mismatch"
    );
    println!(
        "✓ Chrome 147 ciphers: {} entries, order matches profile",
        observed_no_grease.len()
    );
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_chrome147_has_alps_extension() {
    let session = leyline::Session::browser(Browser::default());
    let json = peet(&session).await;

    let extensions = json["tls"]["extensions"]
        .as_array()
        .expect("no tls.extensions");

    let has_alps = extensions.iter().any(|ext| {
        let name = ext["name"].as_str().unwrap_or("");
        name.contains("application_settings") || name.contains("4469") || name.contains("17513")
    });

    assert!(
        has_alps,
        "Chrome 147 should advertise the ALPS extension (application_settings)"
    );
    println!("✓ Chrome 147 ALPS extension present in ClientHello");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_chrome147_has_cert_compression() {
    let session = leyline::Session::browser(Browser::default());
    let json = peet(&session).await;

    let extensions = json["tls"]["extensions"]
        .as_array()
        .expect("no tls.extensions");

    let has_cert_compression = extensions.iter().any(|ext| {
        let name = ext["name"].as_str().unwrap_or("");
        name.contains("compress_certificate") || name.contains("27")
    });

    assert!(
        has_cert_compression,
        "Chrome 147 should advertise compress_certificate extension"
    );
    println!("✓ Chrome 147 compress_certificate extension present");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_firefox_cert_compression_advertises_zlib_brotli_zstd() {
    for browser in [Browser::Firefox150, Browser::Firefox151] {
        let session = leyline::Session::builder()
            .browser(browser)
            .platform(Platform::Windows)
            .build()
            .unwrap();
        let json = peet(&session).await;
        let extensions = json["tls"]["extensions"]
            .as_array()
            .expect("no tls.extensions");

        let cc = extensions
            .iter()
            .find(|ext| {
                let name = ext["name"].as_str().unwrap_or("");
                name.contains("compress_certificate") || name.contains("27")
            })
            .unwrap_or_else(|| panic!("{browser:?}: no compress_certificate extension"));

        let blob = cc.to_string().to_lowercase();
        for algo in ["zlib", "brotli", "zstd"] {
            assert!(
                blob.contains(algo),
                "{browser:?}: compress_certificate must advertise {algo}; got {blob}"
            );
        }
        println!("✓ {browser:?} advertises zlib+brotli+zstd cert compression");
    }
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_chrome147_windows_identity_headers() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .platform(Platform::Windows)
        .build()
        .unwrap();

    let json = peet(&session).await;
    let headers = extract_sent_headers(&json);

    let ua = header_value(&headers, "user-agent").expect("no user-agent");
    assert!(ua.contains("Windows NT 10.0"), "Windows UA wrong: {ua}");
    assert!(ua.contains("Chrome/147.0.0.0"), "UA version wrong: {ua}");

    let platform_raw = header_value(&headers, "sec-ch-ua-platform").expect("no sec-ch-ua-platform");
    let platform = denormalize_peet_quotes(platform_raw);
    assert_eq!(
        platform, "\"Windows\"",
        "Windows platform wrong: {platform}"
    );

    let mobile = header_value(&headers, "sec-ch-ua-mobile").expect("no sec-ch-ua-mobile");
    assert_eq!(
        mobile, "?0",
        "Windows mobile flag should be ?0, got {mobile}"
    );

    println!("✓ Chrome 147 Windows identity headers verified");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_chrome147_android_identity_headers() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .platform(Platform::Android)
        .build()
        .unwrap();

    let json = peet(&session).await;
    let headers = extract_sent_headers(&json);

    let ua = header_value(&headers, "user-agent").expect("no user-agent");
    assert!(
        ua.contains("Android 10; K") && ua.contains("Mobile"),
        "Android UA wrong: {ua}"
    );
    assert!(ua.contains("Chrome/147.0.0.0"), "UA version wrong: {ua}");

    let platform_raw = header_value(&headers, "sec-ch-ua-platform").expect("no sec-ch-ua-platform");
    let platform = denormalize_peet_quotes(platform_raw);
    assert_eq!(
        platform, "\"Android\"",
        "Android platform wrong: {platform}"
    );

    let mobile = header_value(&headers, "sec-ch-ua-mobile").expect("no sec-ch-ua-mobile");
    assert_eq!(
        mobile, "?1",
        "Android mobile flag should be ?1, got {mobile}"
    );

    println!("✓ Chrome 147 Android identity headers verified");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_chrome147_linux_identity_headers() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .platform(Platform::Linux)
        .build()
        .unwrap();

    let json = peet(&session).await;
    let headers = extract_sent_headers(&json);

    let ua = header_value(&headers, "user-agent").expect("no user-agent");
    assert!(ua.contains("X11; Linux x86_64"), "Linux UA wrong: {ua}");

    let platform_raw = header_value(&headers, "sec-ch-ua-platform").expect("no sec-ch-ua-platform");
    let platform = denormalize_peet_quotes(platform_raw);
    assert_eq!(platform, "\"Linux\"", "Linux platform wrong: {platform}");

    let mobile = header_value(&headers, "sec-ch-ua-mobile").expect("no sec-ch-ua-mobile");
    assert_eq!(mobile, "?0", "Linux mobile flag should be ?0, got {mobile}");

    println!("✓ Chrome 147 Linux identity headers verified");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_chrome147_pseudo_header_order() {
    let session = leyline::Session::browser(Browser::default());
    let json = peet(&session).await;
    let headers = extract_sent_headers(&json);

    let pseudos: Vec<&str> = headers
        .iter()
        .filter(|(k, _)| k.starts_with(':'))
        .map(|(k, _)| k.as_str())
        .collect();

    assert_eq!(
        pseudos,
        vec![":method", ":authority", ":scheme", ":path"],
        "Chrome pseudo-header order wrong"
    );
    println!("✓ Chrome 147 pseudo-header order: method,authority,scheme,path");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_firefox150_pseudo_header_order() {
    let session = leyline::Session::builder()
        .browser(Browser::latest(leyline::Family::Firefox))
        .platform(Platform::Windows)
        .build()
        .unwrap();
    let json = peet(&session).await;
    let headers = extract_sent_headers(&json);

    let pseudos: Vec<&str> = headers
        .iter()
        .filter(|(k, _)| k.starts_with(':'))
        .map(|(k, _)| k.as_str())
        .collect();

    assert_eq!(
        pseudos,
        vec![":method", ":path", ":authority", ":scheme"],
        "Firefox pseudo-header order wrong"
    );
    println!("✓ Firefox 150 pseudo-header order: method,path,authority,scheme");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h3_cloudflare() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .protocol(leyline::ProtocolPolicy::Http3)
        .build()
        .expect("h3 session builds");
    let resp = session
        .get("https://cloudflare-quic.com/")
        .await
        .expect("H3 request failed");
    assert_eq!(resp.status(), 200, "H3 status: {}", resp.status());
    assert_eq!(resp.version(), leyline::HttpVersion::Http3, "H3 ALPN");
    assert_eq!(
        resp.tls().and_then(|t| t.version.as_deref()),
        Some("TLSv1.3"),
        "QUIC is always TLS 1.3 (RFC 9001 §4.2)"
    );
    assert!(
        resp.tls()
            .and_then(|t| t.cipher.as_deref())
            .is_some_and(|c| c.starts_with("TLS_") && c.contains("_SHA")),
        "expected a TLS 1.3 cipher suite, got {:?}",
        resp.tls().and_then(|t| t.cipher.as_deref())
    );
    assert!(
        resp.tls()
            .and_then(|t| t.peer_cert_der.as_deref())
            .is_some_and(|c| !c.is_empty()),
        "H3 peer certificate should be exposed"
    );
    let cipher = resp.tls().and_then(|t| t.cipher.clone());
    let body_len = resp.bytes().await.expect("buffered H3 body").len();
    assert!(body_len > 0, "H3 body is empty");
    println!(
        "✓ HTTP/3 to cloudflare-quic.com: status 200, {} bytes, cipher {:?}",
        body_len, cipher
    );
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h3_cloudflare_firefox_profile() {
    let session = leyline::Session::builder()
        .browser(Browser::Firefox150)
        .protocol(leyline::ProtocolPolicy::Http3)
        .build()
        .expect("h3 session builds");
    let resp = session
        .get("https://cloudflare-quic.com/")
        .await
        .expect("H3 request failed (firefox profile)");
    assert_eq!(
        resp.status(),
        200,
        "H3 status with Firefox profile: {}",
        resp.status()
    );
    println!("✓ HTTP/3 to cloudflare-quic.com (Firefox 150 profile): status 200");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h3_google() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .protocol(leyline::ProtocolPolicy::Http3)
        .build()
        .expect("h3 session builds");
    let resp = session
        .request(http::Method::GET, "https://www.google.com/")
        .header("accept", "text/html")
        .send()
        .await
        .expect("H3 request failed");
    assert_eq!(resp.status(), 200, "H3 status: {}", resp.status());
    println!("✓ HTTP/3 to www.google.com: status 200");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h3_pool_reuse() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .protocol(leyline::ProtocolPolicy::Http3)
        .build()
        .expect("h3 session builds");

    let r1 = session
        .get("https://cloudflare-quic.com/")
        .await
        .expect("first H3 request failed");
    assert_eq!(r1.status(), 200);
    drop(r1.bytes().await);
    let after_first = session.pool_stats();

    let r2 = session
        .get("https://cloudflare-quic.com/")
        .await
        .expect("second H3 request failed");
    assert_eq!(r2.status(), 200);
    drop(r2.bytes().await);
    let after_second = session.pool_stats();

    assert_eq!(
        after_first.h3_hits, 0,
        "first request should open a fresh connection, not hit the pool"
    );
    assert!(
        after_second.h3_hits >= 1,
        "second request must reuse the pooled QUIC connection (h3_hits={}, entries={})",
        after_second.h3_hits,
        after_second.entries
    );
    assert_eq!(
        after_second.entries, 1,
        "exactly one pooled H3 connection expected, got {}",
        after_second.entries
    );
    println!(
        "✓ HTTP/3 pool reuse: h3_hits={}, installs={}, entries={}",
        after_second.h3_hits, after_second.installs, after_second.entries
    );
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_race() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .protocol(leyline::ProtocolPolicy::Race)
        .build()
        .expect("race session builds");

    let r1 = session
        .get("https://cloudflare-quic.com/")
        .await
        .expect("first raced request failed");
    assert_eq!(r1.status(), 200, "race status: {}", r1.status());
    drop(r1.bytes().await);

    let after_first = session.pool_stats();
    assert!(
        (1..=2).contains(&after_first.entries),
        "race must warm the winner (and optionally the loser leg), got {} entries \
         (h2_misses={}, h3_misses={})",
        after_first.entries,
        after_first.h2_misses,
        after_first.h3_misses
    );

    let r2 = session
        .get("https://cloudflare-quic.com/")
        .await
        .expect("second raced request failed");
    assert_eq!(r2.status(), 200);
    drop(r2.bytes().await);

    let after_second = session.pool_stats();
    assert!(
        after_second.h2_hits + after_second.h3_hits >= 1,
        "second raced request must reuse the winning connection \
         (h2_hits={}, h3_hits={})",
        after_second.h2_hits,
        after_second.h3_hits
    );
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h3_concurrent_cold_requests_single_flight() {
    use futures_util::future::join_all;

    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .protocol(leyline::ProtocolPolicy::Http3)
        .build()
        .expect("h3 session builds");

    let futs = (0..8).map(|_| session.get("https://cloudflare-quic.com/").send());
    for r in join_all(futs).await {
        assert_eq!(r.expect("concurrent H3 request failed").status(), 200);
    }

    let stats = session.pool_stats();
    assert_eq!(
        stats.installs, 1,
        "8 concurrent cold H3 requests must share ONE handshake, got {} installs \
         (entries={}, h3_hits={}, h3_misses={})",
        stats.installs, stats.entries, stats.h3_hits, stats.h3_misses
    );
    assert_eq!(stats.entries, 1, "exactly one pooled QUIC connection");
    println!(
        "✓ H3 single-flight: installs={}, entries={}, h3_hits={}",
        stats.installs, stats.entries, stats.h3_hits
    );
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h3_response_streaming_is_incremental() {
    use futures_util::StreamExt;

    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .protocol(leyline::ProtocolPolicy::Http3)
        .build()
        .expect("h3 session builds");

    let buffered = session
        .get("https://cloudflare-quic.com/")
        .await
        .expect("buffered H3 request failed");
    let expected = buffered.bytes().await.expect("buffered H3 body").to_vec();

    let resp = session
        .request(http::Method::GET, "https://cloudflare-quic.com/")
        .stream()
        .send()
        .await
        .expect("streamed H3 request failed");
    assert_eq!(resp.status(), 200);
    let encoding = resp.header("content-encoding").map(str::to_ascii_lowercase);

    let mut stream = resp.into_stream().expect("into_stream");
    let mut chunks = 0usize;
    let mut compressed: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.expect("h3 body chunk");
        chunks += 1;
        compressed.extend_from_slice(&chunk);
    }

    let decoded = decode_content_encoding(&compressed, encoding.as_deref());
    assert_eq!(
        decoded,
        expected,
        "streamed H3 body differs from buffered: {} decoded bytes (from {} compressed, encoding={encoding:?}) in {chunks} chunks vs {} buffered",
        decoded.len(),
        compressed.len(),
        expected.len(),
    );
    assert!(
        chunks > 1,
        "expected incremental H3 delivery, got {chunks} chunk(s) for {} bytes",
        compressed.len()
    );
    println!(
        "✓ H3 incremental streaming: {chunks} chunks, {} compressed → {} bytes ({encoding:?})",
        compressed.len(),
        decoded.len()
    );
}

fn decode_content_encoding(body: &[u8], encoding: Option<&str>) -> Vec<u8> {
    use std::io::Read;
    match encoding {
        Some("br") => {
            let mut out = Vec::new();
            brotli::Decompressor::new(body, 4096)
                .read_to_end(&mut out)
                .expect("brotli decode");
            out
        }
        Some("gzip") | Some("x-gzip") => {
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(body)
                .read_to_end(&mut out)
                .expect("gzip decode");
            out
        }
        Some("zstd") => zstd::decode_all(body).expect("zstd decode"),
        Some("deflate") => {
            let mut out = Vec::new();
            flate2::read::ZlibDecoder::new(body)
                .read_to_end(&mut out)
                .expect("deflate decode");
            out
        }
        _ => body.to_vec(),
    }
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_session_resumption_pre_shared_key() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .protocol(leyline::ProtocolPolicy::Http1)
        .build()
        .unwrap();

    let json1 = peet(&session).await;
    let exts1 = json1["tls"]["extensions"].as_array().unwrap();
    let has_psk1 = exts1
        .iter()
        .any(|e| e["name"].as_str().unwrap_or("").contains("pre_shared_key"));
    assert!(
        !has_psk1,
        "first handshake should not carry pre_shared_key (cache is cold)"
    );

    let json2 = peet(&session).await;
    let exts2 = json2["tls"]["extensions"].as_array().unwrap();
    let has_psk2 = exts2
        .iter()
        .any(|e| e["name"].as_str().unwrap_or("").contains("pre_shared_key"));
    assert!(
        has_psk2,
        "second handshake should carry pre_shared_key (ticket was cached)"
    );

    println!("✓ TLS session resumption: first handshake cold, second carries PSK");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h2_connection_reuse() {
    let session = leyline::Session::browser(Browser::default());
    let resp1 = session.get(PEET_URL).await.unwrap();
    assert_eq!(resp1.status(), 200);
    let resp2 = session.get(PEET_URL).await.unwrap();
    assert_eq!(resp2.status(), 200);
    let resp3 = session.get(PEET_URL).await.unwrap();
    assert_eq!(resp3.status(), 200);
    println!("✓ Three sequential requests succeeded (pool reuse)");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_tls_peer_certificate_exposed() {
    let session = leyline::Session::browser(Browser::default());
    let resp = session.get(PEET_URL).await.unwrap();
    assert_eq!(resp.status(), 200);
    let cert = resp
        .tls()
        .and_then(|t| t.peer_cert_der.as_deref())
        .expect("peer certificate should be exposed on HTTPS responses");
    assert!(
        cert.len() > 100 && cert[0] == 0x30,
        "peer cert DER looks malformed: len={}, first={:02x}",
        cert.len(),
        cert.first().copied().unwrap_or(0)
    );
    println!("✓ peer certificate exposed: {} DER bytes", cert.len());
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_websocket_echo() {
    let session = leyline::Session::browser(Browser::default());
    let mut ws = session
        .websocket("wss://ws.postman-echo.com/raw")
        .await
        .expect("ws connect failed");

    ws.send(leyline::WsMessage::Text("leyline-ping".to_owned()))
        .await
        .expect("ws send failed");

    let reply = ws
        .recv()
        .await
        .expect("ws recv failed")
        .expect("ws closed before reply");

    let as_string = format!("{reply:?}");
    assert!(
        as_string.contains("leyline-ping"),
        "unexpected ws reply: {as_string}"
    );

    ws.close().await.expect("ws close failed");
    println!("✓ WebSocket echo: sent=recv=leyline-ping");
}
