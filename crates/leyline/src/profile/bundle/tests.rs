use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::{BundleSource, ProfileBundle};
use crate::profile::{Family, ProfileError};
use crate::{Kind, Session};

const TEST_KEY: &str = "e8a67a2fb400349dacdbbe43ff776d633fe5457cbba44300ae994aa01ad78db0";
const TWO_CHROME: &[u8] = include_bytes!("../../../tests/data/bundle/two-chrome.json");
const FORMAT_2: &[u8] = include_bytes!("../../../tests/data/bundle/format-2.json");

fn test_source(url: &str) -> BundleSource {
    let mut key = [0; 32];
    hex::decode_to_slice(TEST_KEY, &mut key).unwrap();
    BundleSource::new(url, vec![key])
}

fn edit_envelope(edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let mut envelope: serde_json::Value = serde_json::from_slice(TWO_CHROME).unwrap();
    edit(&mut envelope);
    serde_json::to_vec(&envelope).unwrap()
}

async fn serve(body: &'static [u8]) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let _read = sock.read(&mut buf).await.unwrap();
        let head = format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", body.len());
        sock.write_all(head.as_bytes()).await.unwrap();
        sock.write_all(body).await.unwrap();
    });
    addr
}

#[test]
fn a_signed_bundle_loads_its_profiles_and_lists_the_one_it_cannot_parse() {
    let bundle = ProfileBundle::from_bytes(TWO_CHROME, &test_source("unused")).unwrap();

    assert_eq!(bundle.generated(), "2026-10-23T21:00:00-05:00");
    assert_eq!(
        bundle.profiles().get("chrome", 155).unwrap().meta.version,
        155
    );
    assert_eq!(
        bundle
            .profiles()
            .latest(Family::Chrome)
            .unwrap()
            .meta
            .version,
        156
    );
    assert!(bundle.profiles().latest(Family::Firefox).is_none());
    let skipped: Vec<(&str, u32)> = bundle
        .skipped()
        .iter()
        .map(|skip| (skip.browser.as_str(), skip.version))
        .collect();
    assert_eq!(skipped, [("chrome", 999)]);
}

#[test]
fn a_changed_signature_byte_is_rejected() {
    let tampered = edit_envelope(|envelope| {
        let signature = envelope["signature"].as_str().unwrap();
        let flipped = if signature.starts_with('0') { "1" } else { "0" };
        envelope["signature"] = format!("{flipped}{}", &signature[1..]).into();
    });

    let result = ProfileBundle::from_bytes(&tampered, &test_source("unused"));

    assert!(matches!(result, Err(ProfileError::Signature)), "{result:?}");
}

#[test]
fn a_changed_payload_byte_is_rejected() {
    let tampered = edit_envelope(|envelope| {
        let mut payload = STANDARD
            .decode(envelope["payload"].as_str().unwrap())
            .unwrap();
        let middle = payload.len() / 2;
        payload[middle] ^= 1;
        envelope["payload"] = STANDARD.encode(payload).into();
    });

    let result = ProfileBundle::from_bytes(&tampered, &test_source("unused"));

    assert!(matches!(result, Err(ProfileError::Signature)), "{result:?}");
}

#[test]
fn a_bundle_signed_by_an_untrusted_key_is_rejected() {
    let result = ProfileBundle::from_bytes(TWO_CHROME, &BundleSource::default());

    assert!(matches!(result, Err(ProfileError::Signature)), "{result:?}");
}

#[test]
fn a_newer_bundle_format_is_rejected() {
    let result = ProfileBundle::from_bytes(FORMAT_2, &test_source("unused"));

    assert!(
        matches!(result, Err(ProfileError::Format { format: 2 })),
        "{result:?}"
    );
}

#[tokio::test]
async fn fetch_reads_the_served_bundle_through_the_session() {
    let addr = serve(TWO_CHROME).await;
    let session = Session::builder().build().unwrap();

    let bundle = ProfileBundle::fetch(
        &session,
        &test_source(&format!("http://{addr}/profiles.json")),
    )
    .await
    .unwrap();

    assert_eq!(
        bundle
            .profiles()
            .latest(Family::Chrome)
            .unwrap()
            .meta
            .version,
        156
    );
}

#[tokio::test]
async fn fetch_refuses_a_bundle_larger_than_max_bytes() {
    let addr = serve(TWO_CHROME).await;
    let session = Session::builder().build().unwrap();
    let source = test_source(&format!("http://{addr}/profiles.json")).max_bytes(1024);

    let error = ProfileBundle::fetch(&session, &source).await.unwrap_err();

    assert_eq!(error.kind(), Kind::Body);
}
