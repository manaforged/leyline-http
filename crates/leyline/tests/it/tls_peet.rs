#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
#![expect(
    clippy::panic,
    reason = "test harness helper: explicit panic on unexpected error shape is the assertion"
)]
use std::collections::HashMap;
use std::sync::LazyLock;

use leyline::{Browser, Platform};
use serde_json::Value;

static BRAND_JA4: LazyLock<HashMap<String, String>> =
    LazyLock::new(|| toml::from_str(include_str!("../fixtures/brand_ja4.toml")).unwrap());

fn expected_ja4(browser: Browser) -> Option<String> {
    BRAND_JA4.get(browser.id()).cloned().or_else(|| {
        browser
            .branded_profile()
            .tls
            .fingerprint
            .as_ref()
            .and_then(|f| f.ja4.clone())
    })
}

pub(super) const PEET_URL: &str = "https://tls.peet.ws/api/all";

pub(super) async fn peet(session: &leyline::Session) -> Value {
    let mut last_err = String::new();
    for attempt in 0..4 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
        }
        let resp = match session.get(PEET_URL).send().await {
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
        browser.for_platform(platform) == browser && browser.headers(platform).is_some()
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
    let mut checked = 0;

    for browser in Browser::all().iter().copied() {
        let platform = live_platform_for(browser);
        let session = leyline::Session::builder()
            .browser(browser)
            .platform(platform)
            .build()
            .unwrap();
        let json = peet(&session).await;
        let profile = browser.branded_profile();

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

        if let Some(exp) = expected_ja4(browser) {
            assert_eq!(wire_ja4, exp, "{browser} JA4 wire != golden");
        }
        if let Some(exp) = profile
            .h2
            .fingerprint
            .as_ref()
            .and_then(|f| f.akamai.as_deref())
        {
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
    let expected = profile
        .tls
        .fingerprint
        .as_ref()
        .and_then(|f| f.ja4.as_deref())
        .expect("Chrome 149 TOML missing JA4");

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
    let expected = profile
        .tls
        .fingerprint
        .as_ref()
        .and_then(|f| f.ja4.as_deref())
        .expect("Chrome 148 TOML missing JA4");

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
    let expected = profile
        .tls
        .fingerprint
        .as_ref()
        .and_then(|f| f.ja4.as_deref())
        .expect("Chrome 147 TOML missing JA4");

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
    let expected = profile
        .tls
        .fingerprint
        .as_ref()
        .and_then(|f| f.ja4.as_deref())
        .expect("Chrome 146 TOML missing JA4");

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
    let expected = profile
        .tls
        .fingerprint
        .as_ref()
        .and_then(|f| f.ja4.as_deref())
        .expect("Chrome 145 TOML missing JA4");

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
        .tls
        .fingerprint
        .as_ref()
        .and_then(|f| f.ja4.as_deref())
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
        .tls
        .fingerprint
        .as_ref()
        .and_then(|f| f.ja4.as_deref())
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
        .tls
        .fingerprint
        .as_ref()
        .and_then(|f| f.ja4.as_deref())
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
    let expected = profile
        .tls
        .fingerprint
        .as_ref()
        .and_then(|f| f.ja4.as_deref())
        .expect("Safari 18 TOML missing JA4");

    assert_eq!(ja4, expected, "Safari 18 JA4 mismatch");
    println!("✓ Safari 18 JA4 exact match: {ja4}");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_ja4_exact_match_every_profile_with_expectation() {
    let mut checked = 0;
    let mut skipped: Vec<String> = Vec::new();

    for browser in Browser::all().iter().copied() {
        let expected = match expected_ja4(browser) {
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

        let profile = browser.branded_profile();
        let expected = profile
            .h2
            .fingerprint
            .as_ref()
            .and_then(|f| f.akamai.as_deref())
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

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_chrome147_ciphers_match_profile_order() {
    let session = leyline::Session::from(Browser::default());
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
    let session = leyline::Session::from(Browser::default());
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
    let session = leyline::Session::from(Browser::default());
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
    let session = leyline::Session::from(Browser::default());
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
        .browser(Browser::firefox())
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
