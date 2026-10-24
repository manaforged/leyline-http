use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use std::time::UNIX_EPOCH;

use super::{BundleSource, OFFICIAL, ProfileBundle, Wire, check_current};
use crate::profile::{Family, ProfileError};
use crate::{Kind, Session};

const TEST_KEY: &str = include_str!("../../../tests/data/bundle/key.hex");
const TWO_CHROME: &[u8] = include_bytes!("../../../tests/data/bundle/two-chrome.json");
const FORMAT_2: &[u8] = include_bytes!("../../../tests/data/bundle/format-2.json");
const ROLLED_BACK: &[u8] = include_bytes!("../../../tests/data/bundle/rolled-back.json");
const FUTURE_SERIAL: &[u8] = include_bytes!("../../../tests/data/bundle/future-serial.json");
const EXPIRED: &[u8] = include_bytes!("../../../tests/data/bundle/expired.json");

pub(super) fn test_source(url: &str) -> BundleSource {
    let mut key = [0; 32];
    hex::decode_to_slice(TEST_KEY.trim(), &mut key).unwrap();
    BundleSource::new(url, vec![key]).max_lifetime(Duration::from_secs(u64::from(u32::MAX)))
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

    assert_eq!(bundle.serial(), 1_760_000_000);
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

#[test]
fn a_bundle_older_than_the_trusted_serial_is_rejected_even_after_a_lower_floor() {
    let source = test_source("unused")
        .min_serial(1_760_000_000)
        .min_serial(0);

    let result = ProfileBundle::from_bytes(ROLLED_BACK, &source);

    assert!(
        matches!(
            result,
            Err(ProfileError::Rollback {
                serial: 1_750_000_000,
                min_serial: 1_760_000_000
            })
        ),
        "{result:?}"
    );
}

#[test]
fn a_serial_later_than_the_clock_allows_is_rejected() {
    let result = ProfileBundle::from_bytes(FUTURE_SERIAL, &test_source("unused"));

    assert!(
        matches!(
            result,
            Err(ProfileError::FutureSerial {
                serial: 4_000_000_000
            })
        ),
        "{result:?}"
    );
}

#[test]
fn an_expired_bundle_is_rejected() {
    let result = ProfileBundle::from_bytes(EXPIRED, &test_source("unused"));

    assert!(
        matches!(
            result,
            Err(ProfileError::Expired {
                expires: 1_760_000_001
            })
        ),
        "{result:?}"
    );
}

#[tokio::test]
async fn fetch_stops_reading_at_max_bytes_when_the_body_has_no_length() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let _read = sock.read(&mut buf).await.unwrap();
        sock.write_all(b"HTTP/1.1 200 OK\r\nconnection: close\r\n\r\n")
            .await
            .unwrap();
        sock.write_all(&[b'x'; 4096]).await.unwrap();
        std::future::pending::<()>().await;
    });
    let session = Session::builder().build().unwrap();
    let source = test_source(&format!("http://{addr}/profiles.json")).max_bytes(1024);

    let fetched = tokio::time::timeout(
        Duration::from_secs(10),
        ProfileBundle::fetch(&session, &source),
    )
    .await
    .expect("fetch stops at max_bytes instead of waiting for the end of the body");

    assert_eq!(fetched.unwrap_err().kind(), Kind::Body);
}

#[test]
fn a_bundle_that_outlives_the_source_lifetime_is_rejected() {
    let mut key = [0; 32];
    hex::decode_to_slice(TEST_KEY.trim(), &mut key).unwrap();
    let source = BundleSource::new("unused", vec![key]);

    let result = ProfileBundle::from_bytes(TWO_CHROME, &source);

    assert!(
        matches!(
            result,
            Err(ProfileError::Lifetime {
                serial: 1_760_000_000,
                expires: 4_102_444_800
            })
        ),
        "{result:?}"
    );
}

fn wire(serial: u64, expires: u64) -> Wire {
    Wire {
        format: 1,
        serial,
        expires,
        profiles: Vec::new(),
    }
}

#[test]
fn a_serial_equal_to_the_trusted_serial_is_accepted_and_one_less_is_not() {
    let source = test_source("unused").min_serial(1_000);

    assert_eq!(
        check_current(&wire(1_000, 3_000), &source, 2_000).unwrap(),
        UNIX_EPOCH + Duration::from_secs(3_000)
    );
    assert!(matches!(
        check_current(&wire(999, 3_000), &source, 2_000),
        Err(ProfileError::Rollback { serial: 999, .. })
    ));
}

#[test]
fn a_serial_at_the_clock_skew_edge_is_accepted_and_one_past_it_is_not() {
    let source = test_source("unused");
    let edge = 2_000 + OFFICIAL.clock_skew_secs;

    assert_eq!(
        check_current(&wire(edge, edge + 1), &source, 2_000).unwrap(),
        UNIX_EPOCH + Duration::from_secs(edge + 1)
    );
    assert!(matches!(
        check_current(&wire(edge + 1, edge + 2), &source, 2_000),
        Err(ProfileError::FutureSerial { .. })
    ));
}

#[test]
fn a_bundle_is_expired_at_its_expiry_second() {
    let source = test_source("unused");

    assert_eq!(
        check_current(&wire(1_000, 2_001), &source, 2_000).unwrap(),
        UNIX_EPOCH + Duration::from_secs(2_001)
    );
    assert!(matches!(
        check_current(&wire(1_000, 2_000), &source, 2_000),
        Err(ProfileError::Expired { expires: 2_000 })
    ));
}

#[test]
fn a_lifetime_equal_to_the_limit_is_accepted_and_one_second_more_is_not() {
    let source = test_source("unused").max_lifetime(Duration::from_secs(100));

    assert_eq!(
        check_current(&wire(1_950, 2_050), &source, 2_000).unwrap(),
        UNIX_EPOCH + Duration::from_secs(2_050)
    );
    assert!(matches!(
        check_current(&wire(1_950, 2_051), &source, 2_000),
        Err(ProfileError::Lifetime { .. })
    ));
}

#[test]
fn a_loaded_bundle_reports_its_expiry_and_a_source_its_url() {
    let bundle = ProfileBundle::from_bytes(TWO_CHROME, &test_source("unused")).unwrap();

    assert_eq!(
        bundle.expires(),
        UNIX_EPOCH + Duration::from_secs(4_102_444_800)
    );
    assert_eq!(
        test_source("https://example.test/profiles.json").url(),
        "https://example.test/profiles.json"
    );
}
