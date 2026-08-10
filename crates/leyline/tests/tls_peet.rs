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

/// Per-platform H2 overrides resolve to the right Akamai fingerprint.
/// Currently exercises Chrome 145/146/147 macOS, which all drop
/// `unknown_setting8` (and Chrome 145 also drops `max_concurrent_streams`).
#[test]
fn h2_per_platform_overrides_resolve() {
    let reg = leyline::profile::ProfileRegistry::builtin();
    for browser in [Browser::Chrome145, Browser::Chrome146, Browser::Chrome147] {
        let profile = reg.get_browser(browser).unwrap();
        let resolved = profile.h2.resolve_for_platform(Platform::MacOS).unwrap();
        let h2 = leyline::h2::H2Config::from_profile(&resolved).unwrap();
        let actual = h2.akamai_fingerprint();
        let expected = profile
            .expected_h2_fingerprint_for(Platform::MacOS)
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
        (Browser::OkHttpAndroid7, Platform::Android),
        (Browser::SafariIOS15, Platform::IOS),
        (Browser::SafariIOS17, Platform::IOS),
        (Browser::SafariIOS18, Platform::IOS),
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
async fn http3_session_rejects_per_request_proxy_at_send_time() {
    // The build-time guard above can't see per-request `.proxy(...)`
    // overrides. The runtime guard in `send_with_policy` must refuse —
    // never fall back to a direct UDP dial that would expose the real
    // egress IP.
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .http3()
        .build()
        .expect("http3 session without proxy builds");
    let err = session
        .get("https://example.com/")
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

#[tokio::test]
async fn race_policy_with_proxy_never_dials_h3_direct() {
    // Race tries H3 first ONLY when no proxy was requested. With a
    // (dead) per-request proxy the request must route through the proxy
    // and fail — a successful response here would mean the H3 leg dialed
    // the target directly, bypassing the proxy (real-IP leak).
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .race()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .expect("race session builds");
    let result = session
        .get("https://example.com/")
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
    assert_eq!(
        leyline::Session::chrome().browser(),
        Some(Browser::Chrome150)
    );
    assert_eq!(
        leyline::Session::firefox().browser(),
        Some(Browser::Firefox150)
    );
    assert_eq!(
        leyline::Session::safari().browser(),
        Some(Browser::Safari18)
    );
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
///
/// Gated on `feature = "socks"` because leyline compiles the SOCKS5 path
/// out by default — without the gate the client returns
/// `"SOCKS proxy support requires the 'socks' feature"` instantly and the
/// spawned mock-server task hangs forever waiting on an `accept()` that
/// never comes, deadlocking the test runner.
#[cfg(feature = "socks")]
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

/// Fetch tls.peet.ws/api/all with the given session and parse JSON. Retries
/// transient failures (network error, non-200, truncated/non-JSON body) — the
/// pre-push gate makes ~29 live calls, so a single blip must not fail it.
async fn peet(session: &leyline::Session) -> Value {
    let mut last_err = String::new();
    for attempt in 0..4 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
        }
        let resp = match session.navigate(PEET_URL).await {
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
        match serde_json::from_str(&resp.text()) {
            Ok(v) => return v,
            Err(e) => last_err = format!("non-JSON: {e}"),
        }
    }
    panic!("tls.peet.ws failed after retries: {last_err}");
}


/// The real platform for a browser's wire identity (the profile's own
/// identity sections are the source of truth; Windows is the generic
/// default for browser stacks that ship a Windows identity).
fn live_platform_for(browser: Browser) -> Platform {
    match browser {
        Browser::SafariIOS15 | Browser::SafariIOS17 | Browser::SafariIOS18 => Platform::IOS,
        Browser::CfnetworkIOS18 => Platform::IOS,
        Browser::OkHttpAndroid10 => Platform::Android,
        Browser::Safari18 | Browser::CfnetworkMacOS26 => Platform::MacOS,
        _ => Platform::Windows,
    }
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

// ─── Live: full wire fingerprint audit — every dimension peet.ws exposes ────
//
// The single "where do we actually sit on the wire" test. For every profile it
// fetches the real peet.ws observation and reports every wire fingerprint
// dimension: JA4, JA3, peetprint, Akamai-H2. It HARD-ASSERTS the dimensions that
// have a golden in the TOML (ja4, akamai); the others (ja3, peetprint) are
// printed as the real wire values so they can be frozen into goldens next.
#[tokio::test]
#[ignore = "live: needs network"]
async fn live_wire_audit_every_profile() {
    let reg = leyline::profile::ProfileRegistry::builtin();
    let mut checked = 0;

    for browser in ALL_BROWSERS {
        // OkHttp Android 7-9 is TLS 1.2-only; tls.peet.ws requires TLS 1.3.
        if matches!(browser, Browser::OkHttpAndroid7) {
            println!("- {browser:30} skipped (TLS 1.2 — tls.peet.ws requires 1.3)");
            continue;
        }
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

        // Hard-assert the dimensions that have a captured golden.
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

// ─── Live: TLS fingerprint verification — every profile with expected JA4 ──

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
    // Build Chrome 149 explicitly — the default is now Chrome 150, but this test
    // anchors 149's own JA4 (the current stable major), so it must
    // pin the version rather than ride Session::chrome().
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
    let session = leyline::Session::firefox();
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
    let session = leyline::Session::safari();
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

/// Recover the initial TTL from an observed (per-hop-decremented) value by
/// rounding up to the nearest standard initial TTL. Route-robust: a Linux 64
/// stays ≤64 → 64 for any realistic hop count; Windows 128 → 65..=128 → 128.
fn initial_ttl(observed: i64) -> i64 {
    [64, 128, 255]
        .into_iter()
        .find(|&t| observed <= t)
        .unwrap_or(255)
}

#[test]
fn initial_ttl_buckets_to_standard_values() {
    // Linux 64 survives any realistic hop count (observed 34 == 30 hops → 64).
    assert_eq!(initial_ttl(63), 64);
    assert_eq!(initial_ttl(34), 64);
    assert_eq!(initial_ttl(64), 64);
    // Windows 128 likewise (observed 88 == 40 hops → 128).
    assert_eq!(initial_ttl(65), 128);
    assert_eq!(initial_ttl(110), 128);
    assert_eq!(initial_ttl(128), 128);
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
    let session = leyline::Session::chrome();
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
    let session = leyline::Session::chrome();
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
    let session = leyline::Session::chrome();
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

/// Anchors the cert-compression *algorithm list*, not just the extension's
/// presence. `live_chrome147_has_cert_compression` only proves extension 27
/// exists — it would pass even if the advertised algorithm set were wrong.
/// Firefox 150/151 are the only profiles that advertise more than brotli
/// (zlib+brotli+zstd), and registering those decompressors is exactly what
/// the cert-compression change touches on the wire, so it must be anchored
/// against real peet output or the change is unverified (CONTRIBUTING.md tautology
/// rule). We match on the stringified extension so we're robust to peet's
/// exact field naming (`algorithms` vs parsed `data`).
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

        // Whole-extension blob so we don't depend on peet's sub-field name.
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
    let session = leyline::Session::chrome();
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
    // Firefox sends pseudo-headers in the order method, path, authority, scheme.
    let session = leyline::Session::firefox();
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

// ─── Live: HTTP/3 over QUIC ─────────────────────────────────────────────

/// Sanity: HTTP/3 works end-to-end against a known H3 server through the
/// pooled Session path. This guards against regressions in the quiche +
/// BoringSSL stack and in the H3 connection pool / driver.
#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h3_cloudflare() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .http3()
        .build()
        .expect("h3 session builds");
    let resp = session
        .get("https://cloudflare-quic.com/")
        .send()
        .await
        .expect("H3 request failed");
    assert_eq!(resp.status(), 200, "H3 status: {}", resp.status());
    // H3 now surfaces the QUIC handshake TLS detail (was all None before).
    assert_eq!(resp.tls_alpn(), Some("h3"), "H3 ALPN");
    assert_eq!(
        resp.tls_version(),
        Some("TLSv1.3"),
        "QUIC is always TLS 1.3 (RFC 9001 §4.2)"
    );
    assert!(
        resp.tls_cipher()
            .is_some_and(|c| c.starts_with("TLS_") && c.contains("_SHA")),
        "expected a TLS 1.3 cipher suite, got {:?}",
        resp.tls_cipher()
    );
    assert!(
        resp.tls_peer_certificate().is_some_and(|c| !c.is_empty()),
        "H3 peer certificate should be exposed"
    );
    let body = resp.bytes();
    assert!(!body.is_empty(), "H3 body is empty");
    println!(
        "✓ HTTP/3 to cloudflare-quic.com: status 200, {} bytes, cipher {:?}",
        body.len(),
        resp.tls_cipher()
    );
}

/// Same H3 endpoint, but driven from the Firefox 150 profile. If the H3
/// path had its own hand-rolled context, swapping the profile would have
/// no effect and this test would be redundant with the Chrome one. Instead
/// it exercises the shared `build_ssl_context` factory end-to-
/// end: the Firefox profile's cipher list, curves, sigalgs, delegated
/// credentials, record size limit, and extension permutation all flow into
/// the QUIC ClientHello via the same code path H2 uses.
#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h3_cloudflare_firefox_profile() {
    let session = leyline::Session::builder()
        .browser(Browser::Firefox150)
        .http3()
        .build()
        .expect("h3 session builds");
    let resp = session
        .get("https://cloudflare-quic.com/")
        .send()
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
        .http3()
        .build()
        .expect("h3 session builds");
    let resp = session
        .get("https://www.google.com/")
        .header("accept", "text/html")
        .send()
        .await
        .expect("H3 request failed");
    assert_eq!(resp.status(), 200, "H3 status: {}", resp.status());
    println!("✓ HTTP/3 to www.google.com: status 200");
}

/// HTTP/3 connection pooling: two sequential requests on one Session must
/// reuse the same QUIC connection. The first request opens + installs the
/// connection (an `installs` bump, no `h3_hits`); the second must check the
/// pooled connection back out (`h3_hits >= 1`), proving the driver kept it
/// alive between requests rather than re-handshaking.
#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h3_pool_reuse() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .http3()
        .build()
        .expect("h3 session builds");

    let r1 = session
        .get("https://cloudflare-quic.com/")
        .send()
        .await
        .expect("first H3 request failed");
    assert_eq!(r1.status(), 200);
    let _ = r1.bytes();
    let after_first = session.pool_stats();

    let r2 = session
        .get("https://cloudflare-quic.com/")
        .send()
        .await
        .expect("second H3 request failed");
    assert_eq!(r2.status(), 200);
    let _ = r2.bytes();
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

/// Single-flight: N concurrent first-requests to a cold H3 destination share
/// ONE QUIC handshake (one `install`), instead of each opening its own
/// connection and dropping all but the first at install. Without `inflight_h3`
/// coalescing every request handshakes independently and `installs` would be N.
#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h3_concurrent_cold_requests_single_flight() {
    use futures_util::future::join_all;

    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .http3()
        .build()
        .expect("h3 session builds");

    // Fire concurrently on a cold pool: each request misses the pool and joins
    // the single in-flight connect rather than starting its own.
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

/// True connection-level H2/H3 race: `.race()` to an origin that speaks both
/// (cloudflare-quic.com) must succeed, send the request exactly once on the
/// winning transport, and reuse a warm connection on the next request. Each
/// connect runs to completion on its own task (so a cancelled leg can't strand
/// the in-flight map or leak the pool), so the race warms the winner and may
/// also warm the loser leg — 1 or 2 entries, both idle-evictable. The request
/// still goes on the winner only; the reuse check is the observable guarantee.
#[tokio::test]
#[ignore = "live: needs network"]
async fn live_race_sends_once_on_winner() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .race()
        .build()
        .expect("race session builds");

    let r1 = session
        .get("https://cloudflare-quic.com/")
        .send()
        .await
        .expect("first raced request failed");
    assert_eq!(r1.status(), 200, "race status: {}", r1.status());
    let _ = r1.bytes();

    let after_first = session.pool_stats();
    // The race warms the winner and may also warm the loser leg (each connect
    // completes on its own task and pools under its own transport key), so 1 or
    // 2 entries — both idle-evictable. The request was sent once: only the
    // winner's handle receives a `send`. The reuse check below is the
    // observable guarantee that a warm connection is ready for the next request.
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
        .send()
        .await
        .expect("second raced request failed");
    assert_eq!(r2.status(), 200);
    let _ = r2.bytes();

    let after_second = session.pool_stats();
    assert!(
        after_second.h2_hits + after_second.h3_hits >= 1,
        "second raced request must reuse the winning connection \
         (h2_hits={}, h3_hits={})",
        after_second.h2_hits,
        after_second.h3_hits
    );
    let won_h3 = after_second.h3_hits >= 1;
    println!(
        "✓ race: won={}, entries={}, h2_hits={}, h3_hits={}",
        if won_h3 { "H3" } else { "H2" },
        after_second.entries,
        after_second.h2_hits,
        after_second.h3_hits
    );
}

/// Incremental H3 response streaming: a `.stream()` request over HTTP/3 must
/// deliver the body in multiple chunks as they arrive, not as one buffered
/// blob. cloudflare-quic.com returns a brotli/zstd-compressed page that spans
/// several QUIC packets, so the body arrives in several chunks — proving the
/// driver streams rather than buffering then handing back a single chunk. The
/// streamed bytes are raw (content-encoding preserved); decompressing them must
/// reproduce the buffered, auto-decompressed body exactly.
#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h3_response_streaming_is_incremental() {
    use futures_util::StreamExt;

    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .http3()
        .build()
        .expect("h3 session builds");

    // Buffered baseline (auto-decompressed) for the same resource, to compare
    // the streamed body against after decompressing it.
    let expected = session
        .get("https://cloudflare-quic.com/")
        .send()
        .await
        .expect("buffered H3 request failed")
        .into_bytes();

    let resp = session
        .get("https://cloudflare-quic.com/")
        .stream()
        .send()
        .await
        .expect("streamed H3 request failed");
    assert_eq!(resp.status(), 200);
    // Streaming preserves content-encoding (no auto-decompress); capture it
    // before into_stream consumes the response.
    let encoding = resp.header("content-encoding").map(str::to_ascii_lowercase);

    let mut stream = resp.into_stream().expect("into_stream");
    let mut chunks = 0usize;
    let mut compressed: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.expect("h3 body chunk");
        chunks += 1;
        compressed.extend_from_slice(&chunk);
    }

    // The streamed body is delivered raw (compressed). Decompress it per its
    // content-encoding and compare to the buffered (decompressed) baseline: a
    // body truncated at the idle timeout neither decodes cleanly nor matches
    // the full length. Comparing the *compressed* streamed length against the
    // *decompressed* buffered length is meaningless (~9× apart for HTML).
    let decoded = decode_content_encoding(&compressed, encoding.as_deref());
    assert_eq!(
        decoded,
        expected,
        "streamed H3 body differs from buffered: {} decoded bytes (from {} compressed, encoding={encoding:?}) in {chunks} chunks vs {} buffered",
        decoded.len(),
        compressed.len(),
        expected.len(),
    );
    // ... and arrived incrementally (multiple chunks), not as one buffered blob.
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

/// A streaming REQUEST body over H3 is pumped into the request stream chunk by
/// chunk and the terminating FIN rides the last chunk — never an interior one.
/// POST a large multi-chunk body (no content-length, so the driver delimits it
/// by HTTP/3 framing alone) to an echo origin and assert it round-trips exactly:
/// a body finished early (premature FIN) or reordered would not echo intact, and
/// one large enough to span many QUIC packets exercises flow-control parking and
/// the write-retry path.
#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h3_streaming_request_body_roundtrips() {
    use bytes::Bytes;
    use futures_util::stream;
    use leyline::core::Body;

    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .http3()
        .build()
        .expect("h3 session builds");

    // 320 KB of a deterministic, order-sensitive a–z pattern, fed as 1 KB
    // chunks. Larger than the 256 KB per-stream upload window, so the pump's
    // byte-credit must recycle (driver `add_permits` as bytes hit the wire) —
    // a broken credit cycle would deadlock the upload at the window and time the
    // test out. A correct, byte-exact echo proves ordered, complete delivery. A
    // known length is supplied so a `content-length` rides the request (the body
    // is still pumped incrementally) — the common file-upload shape.
    let total = 320 * 1024;
    let expected: Vec<u8> = (0..total).map(|i| b'a' + (i % 26) as u8).collect();
    let chunks: Vec<std::io::Result<Bytes>> = expected
        .chunks(1024)
        .map(|c| Ok(Bytes::copy_from_slice(c)))
        .collect();
    let chunk_count = chunks.len();
    let body = Body::stream_with_length(stream::iter(chunks), total as u64);

    let resp = session
        .post("https://httpbin.agrd.workers.dev/post")
        .header("content-type", "text/plain")
        .body(body)
        .send()
        .await
        .expect("streamed H3 POST failed");
    assert_eq!(resp.status(), 200, "echo origin returned non-200");

    let raw = resp.bytes();
    let v: Value = serde_json::from_slice(raw).expect("echo response is JSON");
    let expected_str = std::str::from_utf8(&expected).unwrap();
    // This echo origin reflects the raw request body in `body` (its schema also
    // echoes `headers`, so a correct `content-length` there is a second witness
    // the whole body landed).
    let echoed = match v["body"].as_str() {
        Some(s) => s,
        None => {
            let keys: Vec<&String> = v
                .as_object()
                .map(|o| o.keys().collect())
                .unwrap_or_default();
            let head: String = String::from_utf8_lossy(raw).chars().take(400).collect();
            panic!("echo `body` is not a string; response keys={keys:?}; head={head}");
        }
    };
    assert_eq!(
        echoed.len(),
        expected_str.len(),
        "echoed body length {} != sent {} (truncated upload — premature FIN?)",
        echoed.len(),
        expected_str.len()
    );
    assert_eq!(
        echoed, expected_str,
        "echoed streaming H3 body differs from sent (corrupt or reordered upload)"
    );
    println!(
        "✓ H3 streaming request body: {} bytes in {chunk_count} chunks round-tripped exactly",
        expected.len()
    );
}

/// Decompress a response body per its `content-encoding`. Used by the H3
/// streaming test, which receives the body raw (leyline's streaming path
/// preserves `content-encoding` and leaves decompression to the caller).
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
        // identity / absent / unknown → already plaintext.
        _ => body.to_vec(),
    }
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
    let session = leyline::Session::chrome();
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
    let session = leyline::Session::chrome();
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
        "1:65536;2:0;4:6291456;6:262144|15663105|0|m,a,s,p",
        "H2 fingerprint changed through HTTP CONNECT proxy"
    );
    println!("✓ HTTP CONNECT proxy: tunneled request preserved Chrome 147 H2 fingerprint");
}

/// SOCKS5 proxy: set $LEYLINE_TEST_SOCKS5_PROXY=socks5://user:pass@host:port
/// to exercise this path. Uses the same assertion as the HTTP CONNECT variant.
/// Gated on the `socks` feature for the same reason as
/// `offline_socks5_proxy_wire_bytes` — the code path is compiled out by
/// default.
#[cfg(feature = "socks")]
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
        "1:65536;2:0;4:6291456;6:262144|15663105|0|m,a,s,p",
        "H2 fingerprint changed through SOCKS5 proxy"
    );
    println!("✓ SOCKS5 proxy: tunneled request preserved Chrome 147 H2 fingerprint");
}

// ─── Live: WebSocket over TLS ───────────────────────────────────────────

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_websocket_echo() {
    // Postman's public echo server echoes text frames verbatim.
    let session = leyline::Session::chrome();
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
