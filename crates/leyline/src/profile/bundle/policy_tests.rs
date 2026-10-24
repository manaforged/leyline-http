use super::ProfileBundle;
use super::tests::test_source;
use crate::profile::ProfileError;

const TLS10: &[u8] = include_bytes!("../../../tests/data/bundle/tls10.json");
const TLS12: &[u8] = include_bytes!("../../../tests/data/bundle/tls12.json");
const MISMATCHED_ENTRY: &[u8] = include_bytes!("../../../tests/data/bundle/mismatched-entry.json");
const WEAK_CIPHER: &[u8] = include_bytes!("../../../tests/data/bundle/weak-cipher.json");
const BORROWED_CIPHER: &[u8] = include_bytes!("../../../tests/data/bundle/borrowed-cipher.json");
const HEADER_INJECTION: &[u8] = include_bytes!("../../../tests/data/bundle/header-injection.json");

fn policy_reason(bundle: &[u8]) -> String {
    let bundle = ProfileBundle::from_bytes(bundle, &test_source("unused")).unwrap();
    assert!(bundle.profiles().get("chrome", 156).is_none());
    match bundle.skipped() {
        [skip] => match &skip.error {
            ProfileError::Policy { reason } => reason.clone(),
            other => panic!("expected a policy refusal, got {other:?}"),
        },
        other => panic!("expected one refused profile, got {other:?}"),
    }
}
#[test]
fn a_remote_profile_cannot_lower_the_minimum_tls_version() {
    assert!(policy_reason(TLS10).contains("min_tls_version \"1.0\""));
}
#[test]
fn a_remote_profile_cannot_add_a_cipher_no_built_in_profile_uses() {
    assert!(policy_reason(WEAK_CIPHER).contains("TLS_RSA_WITH_NULL_SHA256"));
}
#[test]
fn a_remote_profile_cannot_carry_a_line_break_in_a_header_value() {
    assert!(policy_reason(HEADER_INJECTION).contains("user_agent"));
}
#[test]
fn a_remote_profile_cannot_borrow_a_cipher_from_another_browser() {
    assert!(policy_reason(BORROWED_CIPHER).contains("TLS_RSA_WITH_3DES_EDE_CBC_SHA"));
}
#[test]
fn a_remote_profile_at_the_minimum_tls_version_loads() {
    let bundle = ProfileBundle::from_bytes(TLS12, &test_source("unused")).unwrap();

    assert!(bundle.skipped().is_empty(), "{:?}", bundle.skipped());
    assert_eq!(
        bundle
            .profiles()
            .get("chrome", 156)
            .unwrap()
            .tls
            .min_tls_version
            .as_deref(),
        Some("1.2")
    );
}
#[test]
fn a_bundle_entry_must_name_the_profile_it_holds() {
    let bundle = ProfileBundle::from_bytes(MISMATCHED_ENTRY, &test_source("unused")).unwrap();

    assert!(bundle.profiles().get("chrome", 156).is_none());
    match bundle.skipped() {
        [skip] => {
            assert_eq!((skip.browser.as_str(), skip.version), ("chrome", 155));
            assert!(
                matches!(&skip.error, ProfileError::Policy { reason } if reason.contains("holds the profile chrome 156")),
                "{:?}",
                skip.error
            );
        }
        other => panic!("expected one refused entry, got {other:?}"),
    }
}
