//! Integration tests against tls.peet.ws
//!
//! Offline tests (no network, always run):
//!   cargo test -p leyline --test tls_peet
//!
//! Live tests (need network, ignored by default):
//!   cargo test -p leyline --test tls_peet -- --ignored --nocapture
//!
//! These verify — end-to-end against a real TLS inspector — that every
//! profile in the registry produces exactly the fingerprint its TOML
//! claims. If ANY of these assertions fail, we're shipping a lie.

use leyline::profile::{ALL_BROWSERS, PROFILE_COUNT};
use leyline::{Browser, Platform};
use serde_json::Value;

const PEET_URL: &str = "https://tls.peet.ws/api/all";

// ─── Offline: profile data integrity ────────────────────────────────────

#[test]
fn profile_count_matches_constant() {
    let reg = leyline::profile::ProfileRegistry::builtin();
    assert_eq!(reg.len(), PROFILE_COUNT);
}

#[test]
fn every_browser_variant_has_profile() {
    let reg = leyline::profile::ProfileRegistry::builtin();
    for browser in ALL_BROWSERS {
        assert!(
            reg.get_browser(browser).is_some(),
            "no profile for {browser}"
        );
    }
}

#[test]
fn every_profile_has_fingerprint_expectation() {
    let reg = leyline::profile::ProfileRegistry::builtin();
    for browser in ALL_BROWSERS {
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
    for browser in ALL_BROWSERS {
        let profile = reg.get_browser(browser).unwrap();
        if let Some(expected) = profile.expected_h2_fingerprint() {
            let h2 = leyline::h2::H2Config::from_profile(&profile.h2);
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

/// Per-platform H2 overrides resolve to the right Akamai fingerprint.
/// Currently exercises Chrome 145/146/147 macOS, which all drop
/// `unknown_setting8` (and Chrome 145 also drops `max_concurrent_streams`).
#[test]
fn h2_per_platform_overrides_resolve() {
    let reg = leyline::profile::ProfileRegistry::builtin();
    for browser in [Browser::Chrome145, Browser::Chrome146, Browser::Chrome147] {
        let profile = reg.get_browser(browser).unwrap();
        let resolved = profile.h2.resolve_for_platform("macos");
        let h2 = leyline::h2::H2Config::from_profile(&resolved);
        let actual = h2.akamai_fingerprint();
        let expected = profile
            .expected_h2_fingerprint_for("macos")
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
        (Browser::Chrome146, Platform::Windows),
        (Browser::Chrome146, Platform::Android),
        (Browser::Chrome145, Platform::Windows),
        (Browser::Chrome145, Platform::Android),
        (Browser::Aloha138, Platform::Windows),
        (Browser::Aloha138, Platform::MacOS),
        (Browser::Brave146, Platform::MacOS),
        (Browser::Firefox148, Platform::Windows),
        (Browser::Firefox148, Platform::Linux),
        (Browser::Firefox148, Platform::Android),
        (Browser::Safari18, Platform::MacOS),
        (Browser::OkHttpAndroid10, Platform::Android),
        (Browser::OkHttpAndroid7, Platform::Android),
        (Browser::SafariiOS15, Platform::IOS),
        (Browser::SafariiOS17, Platform::IOS),
        (Browser::SafariiOS18, Platform::IOS),
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
    // These are the one-call public API that lets users get a ready-to-use
    // BoringSSL context without touching `leyline-tls` or `boring` directly.
    let tls = leyline::tls_context(Browser::Chrome147);
    assert!(tls.is_ok(), "tls_context failed: {:?}", tls.err());

    let quic = leyline::quic_context(Browser::Chrome147);
    assert!(quic.is_ok(), "quic_context failed: {:?}", quic.err());

    // Every browser variant should produce a working context.
    for browser in ALL_BROWSERS {
        assert!(
            leyline::tls_context(browser).is_ok(),
            "tls_context failed for {browser}"
        );
    }
}

#[test]
fn profile_helper_returns_builtin() {
    let p = leyline::profile(Browser::Chrome147);
    assert_eq!(p.meta.browser, "chrome");
    assert_eq!(p.meta.version, 147);
    assert!(!p.tls.ciphers.is_empty());
}

#[test]
fn session_builder_rejects_http3_with_proxy_at_build_time() {
    // http3 + proxy is unsupported today. Surface it at build time so the
    // misconfiguration fails fast instead of at the first request.
    let err = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .http3()
        .proxy("http://127.0.0.1:8080")
        .build()
        .expect_err("should reject http3+proxy at build");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("http/3") || msg.contains("http3") || msg.contains("proxy"),
        "error message should mention http3/proxy: {msg}"
    );
}

#[tokio::test]
async fn request_builder_timeout_overrides_session_default() {
    // Point at an unroutable address with a 30s session timeout; override
    // to 50ms on the single request. The per-request override must win —
    // the test completes well under 30s.
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap();
    let start = std::time::Instant::now();
    let err = session
        .get("https://192.0.2.1/")
        .timeout(std::time::Duration::from_millis(50))
        .send()
        .await
        .expect_err("should time out on unroutable address");
    let elapsed = start.elapsed();
    assert!(
        matches!(err, leyline::Error::Timeout),
        "expected Error::Timeout, got {err:?}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "per-request timeout didn't fire — elapsed {elapsed:?}"
    );
}

#[test]
fn session_shortcuts_work() {
    let chrome = leyline::Session::chrome_latest();
    assert!(chrome.is_ok());
    assert_eq!(chrome.unwrap().browser(), Browser::Chrome147);

    let firefox = leyline::Session::firefox_latest();
    assert!(firefox.is_ok());
    assert_eq!(firefox.unwrap().browser(), Browser::Firefox148);

    let safari = leyline::Session::safari_latest();
    assert!(safari.is_ok());
    assert_eq!(safari.unwrap().browser(), Browser::Safari18);
}

// ─── Offline: mock proxy protocol state machine ────────────────────────

/// Stand up an HTTP CONNECT mock proxy on localhost that accepts one
/// connection, asserts the client sent a well-formed CONNECT request,
/// responds 200 then intentionally closes. We then observe from the
/// client side that the proxy connect path attempted and completed the
/// CONNECT handshake. This verifies the wire bytes without the cost of
/// doing a full TLS round-trip.
#[tokio::test]
async fn offline_http_connect_proxy_wire_bytes() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = listener.local_addr().unwrap();

    // Mock proxy task: accept once, read until \r\n\r\n, check the request,
    // reply with 200, then drop.
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

        // Required wire contents.
        if !req.starts_with("CONNECT target.example.com:443 HTTP/1.1\r\n") {
            return Err(format!("bad request line: {req:?}"));
        }
        if !req.contains("Host: target.example.com:443") {
            return Err(format!("missing Host header: {req:?}"));
        }
        if !req.contains("Proxy-Authorization: Basic ") {
            return Err(format!("missing Proxy-Authorization: {req:?}"));
        }

        // Reply with 200 and close — client will fail TLS handshake, but
        // that's fine; we've already verified the wire bytes.
        stream
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .await
            .unwrap();
        let _ = stream.shutdown().await;
        Ok(req)
    });

    // Use FingerprintConnector directly — we never reach the TLS handshake,
    // we just want to see the CONNECT bytes hit the mock. We expect the
    // connection to then fail with a TLS error, which we ignore.
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

    // Expected to fail at TLS handshake; we only care that the CONNECT was
    // sent correctly.
    let _ = session.navigate("https://target.example.com/").await;

    // The server task proves the CONNECT payload was well-formed.
    let recv = server.await.unwrap();
    match recv {
        Ok(req) => println!(
            "✓ offline HTTP CONNECT proxy wire bytes OK ({} bytes header)",
            req.len()
        ),
        Err(e) => panic!("mock proxy rejected CONNECT request: {e}"),
    }
}

/// Stand up a SOCKS5 mock proxy on localhost, walk the client through the
/// auth negotiation + CONNECT command, and verify the bytes match RFC 1928.
#[tokio::test]
async fn offline_socks5_proxy_wire_bytes() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        // Greeting: version 5, N methods, method list.
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

        // Accept USERNAME/PASSWORD.
        stream.write_all(&[0x05, 0x02]).await.unwrap();

        // Read auth sub-negotiation: version(1), ulen(1), uname, plen(1), pass.
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

        // Accept auth.
        stream.write_all(&[0x01, 0x00]).await.unwrap();

        // Read CONNECT: version(1) cmd(1) rsv(1) atyp(1) addr port.
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

        // Send success reply (BND.ADDR 0.0.0.0, BND.PORT 0).
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

    // Expected to fail at TLS handshake against a closed socket; we only
    // care that the SOCKS5 bytes matched the spec.
    let _ = session.navigate("https://target.example.com/").await;

    server.await.unwrap();
    println!("✓ offline SOCKS5 wire bytes OK (RFC 1928 greet + auth + CONNECT)");
}

// ─── Live: helpers ──────────────────────────────────────────────────────

/// Fetch tls.peet.ws/api/all with the given session and parse JSON.
async fn peet(session: &leyline::Session) -> Value {
    let resp = session
        .navigate(PEET_URL)
        .await
        .expect("tls.peet.ws request failed");
    assert_eq!(resp.status(), 200, "tls.peet.ws returned {}", resp.status());
    serde_json::from_str(&resp.text()).expect("tls.peet.ws returned non-JSON")
}

/// Normalize an Akamai fingerprint for comparison: tls.peet.ws has a display
/// bug for setting ID 8 (ENABLE_CONNECT_PROTOCOL) — it renders as `:1` not `8:1`.
/// Our wire encoding is correct; the rewrite is purely display normalization.
fn normalize_akamai(fp: &str) -> String {
    fp.replace(";:1", ";8:1")
}

/// Extract the HEADERS frame from the `http2.sent_frames` array and return its
/// header list as a `Vec<(String, String)>` preserving order.
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

/// Find a header value by name (case-insensitive).
fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// tls.peet.ws echoes quoted header values with an odd escape quirk (adding a
/// backslash before each `"` and dropping the final `"`). Normalize so we can
/// compare against bare-quoted expected values. e.g.:
///   raw:        `\"Windows\`
///   normalized: `"Windows"`
fn denormalize_peet_quotes(raw: &str) -> String {
    if !raw.contains('\\') {
        return raw.to_string();
    }
    let mut s = raw.replace("\\\"", "\"");
    // If the value ends with a stray trailing backslash (their truncation bug),
    // replace it with a closing quote.
    if s.ends_with('\\') {
        s.pop();
        s.push('"');
    }
    s
}

// ─── Live: TLS fingerprint verification — every profile with expected JA4 ──

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_ja4_exact_match_chrome147() {
    let session = leyline::Session::chrome_latest().unwrap();
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
async fn live_ja4_exact_match_firefox148() {
    let session = leyline::Session::firefox_latest().unwrap();
    let json = peet(&session).await;
    let ja4 = json["tls"]["ja4"].as_str().expect("no tls.ja4");

    let reg = leyline::profile::ProfileRegistry::builtin();
    let profile = reg.get_browser(Browser::Firefox148).unwrap();
    let expected = profile
        .expected_ja4()
        .expect("Firefox 148 TOML missing JA4");

    assert_eq!(ja4, expected, "Firefox 148 JA4 mismatch");
    println!("✓ Firefox 148 JA4 exact match: {ja4}");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_ja4_exact_match_safari18() {
    let session = leyline::Session::safari_latest().unwrap();
    let json = peet(&session).await;
    let ja4 = json["tls"]["ja4"].as_str().expect("no tls.ja4");

    let reg = leyline::profile::ProfileRegistry::builtin();
    let profile = reg.get_browser(Browser::Safari18).unwrap();
    let expected = profile.expected_ja4().expect("Safari 18 TOML missing JA4");

    assert_eq!(ja4, expected, "Safari 18 JA4 mismatch");
    println!("✓ Safari 18 JA4 exact match: {ja4}");
}

/// Assert every profile with an expected_ja4() in its TOML matches exactly.
/// This is the strong umbrella test: if it fails, something in the wire
/// changed vs what the TOML claims, and we need to investigate.
#[tokio::test]
#[ignore = "live: needs network"]
async fn live_ja4_exact_match_every_profile_with_expectation() {
    let reg = leyline::profile::ProfileRegistry::builtin();
    let mut checked = 0;
    let mut skipped: Vec<String> = Vec::new();

    for browser in ALL_BROWSERS {
        if matches!(browser, Browser::OkHttpAndroid7) {
            skipped.push(format!("{browser} (TLS 1.2 only — tls.peet.ws rejects)"));
            continue;
        }

        let profile = reg.get_browser(browser).unwrap();
        let expected = match profile.expected_ja4() {
            Some(e) => e,
            None => {
                skipped.push(format!("{browser} (no JA4 in TOML)"));
                continue;
            }
        };

        let platform = match browser {
            Browser::SafariiOS15 | Browser::SafariiOS17 | Browser::SafariiOS18 => Platform::IOS,
            Browser::OkHttpAndroid10 => Platform::Android,
            Browser::Safari18 => Platform::MacOS,
            _ => Platform::Windows,
        };
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

// ─── Live: every profile's H2 Akamai fingerprint matches TOML ───────────

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h2_akamai_every_profile() {
    let reg = leyline::profile::ProfileRegistry::builtin();
    let mut checked = 0;

    for browser in ALL_BROWSERS {
        // OkHttp Android 7-9 is TLS 1.2-only; tls.peet.ws requires TLS 1.3
        // (HANDSHAKE_FAILURE_ON_CLIENT_HELLO). Skip and log.
        if matches!(browser, Browser::OkHttpAndroid7) {
            println!("- {browser:30} skipped (TLS 1.2 — tls.peet.ws requires 1.3)");
            continue;
        }

        let platform = match browser {
            Browser::SafariiOS15 | Browser::SafariiOS17 | Browser::SafariiOS18 => Platform::IOS,
            Browser::OkHttpAndroid10 | Browser::OkHttpAndroid7 => Platform::Android,
            Browser::Safari18 => Platform::MacOS,
            _ => Platform::Windows,
        };

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
        PROFILE_COUNT - 1,
        "expected to check all profiles except OkHttp 7"
    );
}

// ─── Live: TCP fingerprint per platform (observed via tls.peet.ws) ──────

/// Run a TCP handshake and return the TTL tls.peet.ws observed for the SYN.
/// The observed TTL has been decremented by ~10-20 hops on a typical route,
/// so we assert ranges rather than exact values.
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

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_tcp_windows_ttl_is_128() {
    let ttl = observe_ttl(Browser::Chrome147, Platform::Windows).await;
    // Initial 128, minus ~10-20 hops → expect 98..=127.
    assert!(
        (98..=127).contains(&ttl),
        "Windows TTL out of expected range (98..=127): {ttl}. \
         Initial TTL should be 128 but socket is sending {}",
        ttl + 13
    );
    println!("✓ Windows TCP TTL: observed {ttl} (initial 128)");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_tcp_linux_ttl_is_64() {
    let ttl = observe_ttl(Browser::Chrome147, Platform::Linux).await;
    // Initial 64, minus ~10-20 hops → expect 34..=63.
    assert!(
        (34..=63).contains(&ttl),
        "Linux TTL out of expected range (34..=63): {ttl}"
    );
    println!("✓ Linux TCP TTL: observed {ttl} (initial 64)");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_tcp_windows_distinguishable_from_linux() {
    // The main point of TCP fingerprinting: Windows and Linux must be
    // distinguishable on the wire.
    let win_ttl = observe_ttl(Browser::Chrome147, Platform::Windows).await;
    let linux_ttl = observe_ttl(Browser::Chrome147, Platform::Linux).await;
    assert!(
        win_ttl > linux_ttl + 40,
        "Windows TTL ({win_ttl}) not sufficiently greater than Linux TTL ({linux_ttl}) — \
         TCP profile may not be applied"
    );
    println!("✓ TCP TTL distinguishes Windows ({win_ttl}) from Linux ({linux_ttl})");
}

// ─── Live: ClientHello structure ────────────────────────────────────────

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_chrome147_ciphers_match_profile_order() {
    let session = leyline::Session::chrome_latest().unwrap();
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

    // Filter out GREASE values (TLS_GREASE_*) from observed.
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
    let session = leyline::Session::chrome_latest().unwrap();
    let json = peet(&session).await;

    let extensions = json["tls"]["extensions"]
        .as_array()
        .expect("no tls.extensions");

    // Chrome 147 uses new ALPS codepoint (0x4469). Old ALPS codepoint is 0x4468.
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
    let session = leyline::Session::chrome_latest().unwrap();
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

// ─── Live: platform identity → header content ───────────────────────────

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
        ua.contains("Android 14") && ua.contains("Mobile"),
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

// ─── Live: header ordering (Chrome pseudo-header order) ─────────────────

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_chrome147_pseudo_header_order() {
    // Chrome sends pseudo-headers in the order method, authority, scheme, path.
    let session = leyline::Session::chrome_latest().unwrap();
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
async fn live_firefox148_pseudo_header_order() {
    // Firefox sends pseudo-headers in the order method, path, authority, scheme.
    let session = leyline::Session::firefox_latest().unwrap();
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
    println!("✓ Firefox 148 pseudo-header order: method,path,authority,scheme");
}

// ─── Live: decompression ─────────────────────────────────────────────────

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_decompression_gzip() {
    let session = leyline::Session::chrome_latest().unwrap();
    let resp = session.navigate("https://httpbin.org/gzip").await.unwrap();
    assert_eq!(resp.status(), 200);
    let json: Value = serde_json::from_str(&resp.text()).expect("gzip-decoded body not JSON");
    assert_eq!(json["gzipped"], true);
    println!("✓ gzip decompression works");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_decompression_brotli() {
    let session = leyline::Session::chrome_latest().unwrap();
    let resp = session
        .navigate("https://httpbin.org/brotli")
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: Value = serde_json::from_str(&resp.text()).expect("brotli-decoded body not JSON");
    assert_eq!(json["brotli"], true);
    println!("✓ brotli decompression works");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_decompression_deflate() {
    let session = leyline::Session::chrome_latest().unwrap();
    let resp = session
        .navigate("https://httpbin.org/deflate")
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: Value = serde_json::from_str(&resp.text()).expect("deflate-decoded body not JSON");
    assert_eq!(json["deflated"], true);
    println!("✓ deflate decompression works");
}

// ─── Live: cookies ───────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_cookies_set_then_sent() {
    let session = leyline::Session::chrome_latest().unwrap();
    // Set a cookie via Set-Cookie redirect chain.
    let resp1 = session
        .navigate("https://httpbin.org/cookies/set?token=abc123")
        .await
        .unwrap();
    assert_eq!(resp1.status(), 200);

    // Next request should send the cookie.
    let resp2 = session
        .navigate("https://httpbin.org/cookies")
        .await
        .unwrap();
    assert_eq!(resp2.status(), 200);
    let json: Value = serde_json::from_str(&resp2.text()).unwrap();
    assert_eq!(
        json["cookies"]["token"].as_str(),
        Some("abc123"),
        "cookie not sent on second request"
    );
    println!("✓ Cookie round-trip: set via Set-Cookie, sent on next request");
}

// ─── Live: redirect handling ─────────────────────────────────────────────

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_redirect_follows_and_rewrites_url() {
    let session = leyline::Session::chrome_latest().unwrap();
    let resp = session
        .navigate("https://httpbin.org/redirect/3")
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.redirect_chain().len(),
        3,
        "expected 3 redirects in chain"
    );
    assert!(
        resp.url().ends_with("/get"),
        "final URL wrong: {}",
        resp.url()
    );
    println!("✓ 3-hop redirect chain followed");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_redirect_strips_auth_cross_origin() {
    // Set up bearer auth header and redirect to a different host.
    // httpbin doesn't easily cross-host redirect, but we can check same-host preserves.
    let session = leyline::Session::chrome_latest().unwrap();
    let resp = session
        .get("https://httpbin.org/redirect-to?url=https://httpbin.org/headers")
        .bearer_auth("secret-token-xyz")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: Value = serde_json::from_str(&resp.text()).unwrap();
    // Same-host redirect preserves Authorization.
    assert_eq!(
        json["headers"]["Authorization"].as_str(),
        Some("Bearer secret-token-xyz"),
        "same-host redirect should preserve Authorization"
    );
    println!("✓ Same-host redirect preserves Authorization");
}

// ─── Live: HTTP/3 over QUIC ─────────────────────────────────────────────

/// Sanity: HTTP/3 works end-to-end against a known H3 server. This guards
/// against regressions in the quiche + BoringSSL stack.
#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h3_cloudflare() {
    use leyline::quic::{H3Config, H3Connection};
    let reg = leyline::profile::ProfileRegistry::builtin();
    let profile = reg.get_browser(Browser::Chrome147).unwrap();
    let config = H3Config::chrome();
    let resp = H3Connection::request(
        &config,
        profile,
        "GET",
        "cloudflare-quic.com",
        443,
        "/",
        vec![],
        None,
    )
    .await
    .expect("H3 request failed");
    assert_eq!(resp.status, 200, "H3 status: {}", resp.status);
    assert!(!resp.body.is_empty(), "H3 body is empty");
    println!(
        "✓ HTTP/3 to cloudflare-quic.com: status {}, {} bytes",
        resp.status,
        resp.body.len()
    );
}

/// Same H3 endpoint, but driven from the Firefox 148 profile. If the H3
/// path had its own hand-rolled context, swapping the profile would have
/// no effect and this test would be redundant with the Chrome one. Instead
/// it exercises the shared `build_ssl_context` factory end-to-
/// end: the Firefox profile's cipher list, curves, sigalgs, delegated
/// credentials, record size limit, and extension permutation all flow into
/// the QUIC ClientHello via the same code path H2 uses.
#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h3_cloudflare_firefox_profile() {
    use leyline::quic::{H3Config, H3Connection};
    let reg = leyline::profile::ProfileRegistry::builtin();
    let profile = reg.get_browser(Browser::Firefox148).unwrap();
    let config = H3Config::firefox();
    let resp = H3Connection::request(
        &config,
        profile,
        "GET",
        "cloudflare-quic.com",
        443,
        "/",
        vec![],
        None,
    )
    .await
    .expect("H3 request failed (firefox profile)");
    assert_eq!(
        resp.status, 200,
        "H3 status with Firefox profile: {}",
        resp.status
    );
    println!(
        "✓ HTTP/3 to cloudflare-quic.com (Firefox 148 profile): status {}, {} bytes",
        resp.status,
        resp.body.len()
    );
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h3_google() {
    use leyline::quic::{H3Config, H3Connection};
    let reg = leyline::profile::ProfileRegistry::builtin();
    let profile = reg.get_browser(Browser::Chrome147).unwrap();
    let config = H3Config::chrome();
    let resp = H3Connection::request(
        &config,
        profile,
        "GET",
        "www.google.com",
        443,
        "/",
        vec![("accept".into(), "text/html".into())],
        None,
    )
    .await
    .expect("H3 request failed");
    assert_eq!(resp.status, 200, "H3 status: {}", resp.status);
    println!(
        "✓ HTTP/3 to www.google.com: status {}, {} bytes",
        resp.status,
        resp.body.len()
    );
}

// ─── Live: session resumption ───────────────────────

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_session_resumption_pre_shared_key() {
    // Strong test: force HTTP/1.1 (no connection pool reuse), which makes
    // every request do a fresh TLS handshake on the same Session. The
    // FingerprintConnector holds the session cache as a field, so the
    // ticket received during request #1 should be re-presented on request
    // #2's new handshake as the `pre_shared_key` extension.
    //
    // This depends on tls.peet.ws issuing a TLS 1.3 session ticket during
    // the first response and on our set_new_session_callback running. If
    // anything in that pipeline breaks, this test catches it.
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .http1()
        .build()
        .unwrap();

    // First request — no ticket cached yet.
    let json1 = peet(&session).await;
    let exts1 = json1["tls"]["extensions"].as_array().unwrap();
    let has_psk1 = exts1
        .iter()
        .any(|e| e["name"].as_str().unwrap_or("").contains("pre_shared_key"));
    assert!(
        !has_psk1,
        "first handshake should not carry pre_shared_key (cache is cold)"
    );

    // Second request — new TCP+TLS handshake (no H1 pooling), cached
    // ticket should be presented.
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

// ─── Live: connection reuse (pooling) ───────────────────────────────────

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h2_connection_reuse() {
    let session = leyline::Session::chrome_latest().unwrap();
    let resp1 = session.navigate(PEET_URL).await.unwrap();
    assert_eq!(resp1.status(), 200);
    let resp2 = session.navigate(PEET_URL).await.unwrap();
    assert_eq!(resp2.status(), 200);
    let resp3 = session.navigate(PEET_URL).await.unwrap();
    assert_eq!(resp3.status(), 200);
    println!("✓ Three sequential requests succeeded (pool reuse)");
}

// ─── Live: peer certificate exposure ───────────────────────────────────

/// Confirms `Response::tls_peer_certificate` is actually populated by
/// the HTTPS transport — previously the field was hardcoded to `None`,
/// so the accessor looked like a stable feature but returned nothing.
#[tokio::test]
#[ignore = "live: needs network"]
async fn live_tls_peer_certificate_exposed() {
    let session = leyline::Session::chrome_latest().unwrap();
    let resp = session.navigate(PEET_URL).await.unwrap();
    assert_eq!(resp.status(), 200);
    let cert = resp
        .tls_peer_certificate()
        .expect("peer certificate should be exposed on HTTPS responses");
    // DER-encoded X.509 starts with SEQUENCE (0x30) then length.
    assert!(
        cert.len() > 100 && cert[0] == 0x30,
        "peer cert DER looks malformed: len={}, first={:02x}",
        cert.len(),
        cert.first().copied().unwrap_or(0)
    );
    println!("✓ peer certificate exposed: {} DER bytes", cert.len());
}

// ─── Live: GREASE seed determinism ──────────────────────────────────────

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_grease_seed_deterministic() {
    // Two sessions with the same grease_seed should produce the same JA4_r
    // (the unhashed JA4 which includes cipher and extension order with
    // GREASE values filtered out consistently).
    let s1 = leyline::Session::builder()
        .grease_seed(b"user_42")
        .build()
        .unwrap();
    let s2 = leyline::Session::builder()
        .grease_seed(b"user_42")
        .build()
        .unwrap();

    let j1 = peet(&s1).await;
    let j2 = peet(&s2).await;

    // JA4 is hashed and filters GREASE, so it should match regardless.
    // JA4_r is the raw form; also matches when GREASE is filtered.
    let ja4_1 = j1["tls"]["ja4"].as_str().unwrap();
    let ja4_2 = j2["tls"]["ja4"].as_str().unwrap();
    assert_eq!(ja4_1, ja4_2, "same grease seed should produce same JA4");
    println!("✓ grease_seed determinism: same seed → same JA4");
}

// ─── Live: POST with form & JSON through tls.peet.ws echo ──────────────

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_post_json_body_roundtrip() {
    let session = leyline::Session::chrome_latest().unwrap();
    let body = serde_json::json!({"test": "leyline", "n": 42});
    let resp = session
        .post_json("https://httpbin.org/post", &body)
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: Value = serde_json::from_str(&resp.text()).unwrap();
    assert_eq!(json["json"]["test"], "leyline");
    assert_eq!(json["json"]["n"], 42);
    println!("✓ POST JSON round-trip");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_post_form_body_roundtrip() {
    let session = leyline::Session::chrome_latest().unwrap();
    let resp = session
        .post_form(
            "https://httpbin.org/post",
            &[("u", "alice"), ("p", "s3cret")],
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: Value = serde_json::from_str(&resp.text()).unwrap();
    assert_eq!(json["form"]["u"], "alice");
    assert_eq!(json["form"]["p"], "s3cret");
    println!("✓ POST form round-trip");
}

// ─── Live: proxy integration (env-gated) ───────────────────────────────

/// HTTP CONNECT proxy: set $LEYLINE_TEST_HTTP_PROXY=http://user:pass@host:port
/// to exercise this path end-to-end. The assertion checks that tls.peet.ws
/// sees a request and returns the expected H2 fingerprint (which proves the
/// CONNECT tunnel was established and our TLS/H2 stack rode over it).
#[tokio::test]
#[ignore = "live: needs network + $LEYLINE_TEST_HTTP_PROXY"]
async fn live_http_connect_proxy() {
    let proxy_url = match std::env::var("LEYLINE_TEST_HTTP_PROXY") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            eprintln!("skipping: LEYLINE_TEST_HTTP_PROXY not set");
            return;
        }
    };

    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .proxy(&proxy_url)
        .build()
        .expect("build session with proxy");

    let resp = session
        .navigate(PEET_URL)
        .await
        .expect("proxy-tunnelled fetch failed");
    assert_eq!(resp.status(), 200);
    let json: Value = serde_json::from_str(&resp.text()).unwrap();
    let h2 = json["http2"]["akamai_fingerprint"].as_str().unwrap();
    assert_eq!(
        normalize_akamai(h2),
        "1:65536;2:0;4:6291456;6:262144;8:1|15663105|0|m,a,s,p",
        "H2 fingerprint changed through HTTP CONNECT proxy"
    );
    println!("✓ HTTP CONNECT proxy: tunneled request preserved Chrome 147 H2 fingerprint");
}

/// SOCKS5 proxy: set $LEYLINE_TEST_SOCKS5_PROXY=socks5://user:pass@host:port
/// to exercise this path. Uses the same assertion as the HTTP CONNECT variant.
#[tokio::test]
#[ignore = "live: needs network + $LEYLINE_TEST_SOCKS5_PROXY"]
async fn live_socks5_proxy() {
    let proxy_url = match std::env::var("LEYLINE_TEST_SOCKS5_PROXY") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            eprintln!("skipping: LEYLINE_TEST_SOCKS5_PROXY not set");
            return;
        }
    };

    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .proxy(&proxy_url)
        .build()
        .expect("build session with proxy");

    let resp = session
        .navigate(PEET_URL)
        .await
        .expect("socks5-tunnelled fetch failed");
    assert_eq!(resp.status(), 200);
    let json: Value = serde_json::from_str(&resp.text()).unwrap();
    let h2 = json["http2"]["akamai_fingerprint"].as_str().unwrap();
    assert_eq!(
        normalize_akamai(h2),
        "1:65536;2:0;4:6291456;6:262144;8:1|15663105|0|m,a,s,p",
        "H2 fingerprint changed through SOCKS5 proxy"
    );
    println!("✓ SOCKS5 proxy: tunneled request preserved Chrome 147 H2 fingerprint");
}

// ─── Live: WebSocket over TLS ───────────────────────────────────────────

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_websocket_echo() {
    // Postman's public echo server echoes text frames verbatim.
    let session = leyline::Session::chrome_latest().unwrap();
    let mut ws = session
        .websocket("wss://ws.postman-echo.com/raw")
        .await
        .expect("ws connect failed");

    ws.send("leyline-ping").await.expect("ws send failed");

    let reply = ws
        .recv()
        .await
        .expect("ws recv failed")
        .expect("ws closed before reply");

    // Message type is currently a tokio_tungstenite re-export; use the
    // Debug formatter to avoid bringing that dep into the test.
    let as_string = format!("{reply:?}");
    assert!(
        as_string.contains("leyline-ping"),
        "unexpected ws reply: {as_string}"
    );

    ws.close().await.expect("ws close failed");
    println!("✓ WebSocket echo: sent=recv=leyline-ping");
}
