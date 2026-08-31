//! Live fingerprint gate for the CFNetwork (Apple URLSession) profile family.

#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
#![expect(
    clippy::panic,
    reason = "test harness helper: explicit panic on unexpected error shape is the assertion"
)]
use leyline::{Browser, Platform, Session};
use serde_json::Value;

const PEET_URL: &str = "https://tls.peet.ws/api/all";

async fn peet(session: &Session) -> Value {
    let resp = session.get(PEET_URL).await.expect("navigate failed");
    assert_eq!(resp.status(), 200, "status {}", resp.status());
    serde_json::from_str(&resp.text().unwrap()).expect("non-JSON body")
}

fn ext<'a>(json: &'a Value, prefix: &str) -> &'a Value {
    json["tls"]["extensions"]
        .as_array()
        .expect("extensions")
        .iter()
        .find(|e| e["name"].as_str().unwrap_or_default().starts_with(prefix))
        .unwrap_or_else(|| panic!("missing extension {prefix}"))
}

fn ciphers_without_grease(json: &Value) -> Vec<&str> {
    json["tls"]["ciphers"]
        .as_array()
        .expect("ciphers")
        .iter()
        .map(|c| c.as_str().unwrap())
        .filter(|c| !c.starts_with("TLS_GREASE"))
        .collect()
}

/// Shared CFNetwork shape, asserted for every cfnetwork profile.
fn assert_cfnetwork_common(json: &Value) {
    let ciphers = json["tls"]["ciphers"].as_array().expect("ciphers");
    assert!(
        ciphers[0].as_str().unwrap().starts_with("TLS_GREASE"),
        "GREASE cipher must be first, got {}",
        ciphers[0]
    );
    let exts = json["tls"]["extensions"].as_array().expect("exts");
    assert!(exts[0]["name"].as_str().unwrap().starts_with("TLS_GREASE"));
    let last = exts.iter().rev().find(|e| {
        let n = e["name"].as_str().unwrap_or_default();
        n.starts_with("TLS_GREASE")
    });
    assert!(last.is_some(), "trailing GREASE extension missing");

    let sigalgs = ext(json, "signature_algorithms")["signature_algorithms"]
        .as_array()
        .expect("sigalgs");
    let dup_count = sigalgs
        .iter()
        .filter(|s| s.as_str() == Some("rsa_pss_rsae_sha384"))
        .count();
    assert_eq!(dup_count, 2, "CFNetwork duplicates 0x0805 in sigalgs");

    let cc = ext(json, "compress_certificate");
    assert!(
        cc["algorithms"]
            .as_array()
            .is_some_and(|a| a[0].as_str() == Some("zlib (1)")),
        "cert compression must be zlib"
    );

    assert!(
        !exts.iter().any(|e| e["name"]
            .as_str()
            .unwrap_or_default()
            .contains("session_ticket")),
        "CFNetwork sends no session_ticket extension"
    );

    assert_eq!(
        json["http2"]["akamai_fingerprint"]
            .as_str()
            .unwrap()
            .rsplit('|')
            .next()
            .unwrap(),
        "m,s,p,a"
    );
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_cfnetwork_macos26_matches_capture() {
    let session = Session::builder()
        .browser(Browser::CfnetworkMacOS26)
        .platform(Platform::MacOS)
        .build()
        .unwrap();
    let json = peet(&session).await;

    assert_cfnetwork_common(&json);
    assert_eq!(
        json["tls"]["ja4"].as_str().unwrap(),
        "t13d2013h2_a09f3c656075_7f0f34a4126d"
    );
    assert_eq!(
        json["http2"]["akamai_fingerprint"].as_str().unwrap(),
        "2:0;4:4194304;3:100;9:1|10485760|0|m,s,p,a"
    );
    assert!(
        !json["tls"]["extensions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["name"]
                .as_str()
                .unwrap_or_default()
                .starts_with("padding")),
        "macOS CFNetwork sends no padding extension"
    );
    assert!(
        ext(&json, "supported_groups")["supported_groups"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g.as_str().unwrap().contains("MLKEM768"))
    );
    assert_eq!(
        &ciphers_without_grease(&json)[..3],
        &[
            "TLS_AES_256_GCM_SHA384",
            "TLS_CHACHA20_POLY1305_SHA256",
            "TLS_AES_128_GCM_SHA256",
        ]
    );
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_cfnetwork_ios18_matches_capture() {
    let session = Session::builder()
        .browser(Browser::CfnetworkIOS18)
        .platform(Platform::IOS)
        .build()
        .unwrap();
    let json = peet(&session).await;

    assert_cfnetwork_common(&json);
    assert_eq!(
        json["tls"]["ja4"].as_str().unwrap(),
        "t13d2014h2_a09f3c656075_7f0f34a4126d"
    );
    assert_eq!(
        json["http2"]["akamai_fingerprint"].as_str().unwrap(),
        "2:0;4:2097152;3:100|10485760|0|m,s,p,a"
    );
    let groups = ext(&json, "supported_groups")["supported_groups"]
        .as_array()
        .unwrap();
    assert!(
        !groups
            .iter()
            .any(|g| g.as_str().unwrap().contains("MLKEM768")),
        "iOS 18.6 CFNetwork has no MLKEM group"
    );
    assert_eq!(
        &ciphers_without_grease(&json)[..3],
        &[
            "TLS_AES_128_GCM_SHA256",
            "TLS_AES_256_GCM_SHA384",
            "TLS_CHACHA20_POLY1305_SHA256",
        ]
    );
    let versions = ext(&json, "supported_versions")["versions"]
        .as_array()
        .unwrap();
    assert!(
        versions.iter().any(|v| v.as_str() == Some("TLS 1.0"))
            && versions.iter().any(|v| v.as_str() == Some("TLS 1.1"))
    );
    let exts = json["tls"]["extensions"].as_array().unwrap();
    let padding = exts.last().expect("last ext");
    assert!(
        padding["name"].as_str().unwrap().starts_with("padding"),
        "padding must be the last extension"
    );
    assert_eq!(padding["padding_data_length"].as_u64(), Some(394));
}
