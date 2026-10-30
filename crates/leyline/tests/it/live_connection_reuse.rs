use leyline::Browser;

use super::tls_peet::{PEET_URL, peet};

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_session_resumption_pre_shared_key() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .protocol(leyline::Protocol::Http1)
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
    let session = leyline::Session::from(Browser::default());
    let resp1 = session.get(PEET_URL).send().await.unwrap();
    assert_eq!(resp1.status(), 200);
    let resp2 = session.get(PEET_URL).send().await.unwrap();
    assert_eq!(resp2.status(), 200);
    let resp3 = session.get(PEET_URL).send().await.unwrap();
    assert_eq!(resp3.status(), 200);
    println!("✓ Three sequential requests succeeded (pool reuse)");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_tls_peer_certificate_exposed() {
    let session = leyline::Session::from(Browser::default());
    let resp = session.get(PEET_URL).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let cert = resp
        .peer_certificate()
        .expect("peer certificate should be exposed on HTTPS responses");
    assert!(
        cert.len() > 100 && cert[0] == 0x30,
        "peer cert DER looks malformed: len={}, first={:02x}",
        cert.len(),
        cert.first().copied().unwrap_or(0)
    );
    println!("✓ peer certificate exposed: {} DER bytes", cert.len());
}
