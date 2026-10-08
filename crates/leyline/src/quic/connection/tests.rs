use super::{check_body_budget, resolve_peer, validate_connection_id_len};
use crate::profile::BrowserProfile;
use crate::quic::config::H3Config;
use crate::tls::TlsTrustConfig;
use leyline_bssl::asn1::Asn1Time;
use leyline_bssl::bn::BigNum;
use leyline_bssl::hash::MessageDigest;
use leyline_bssl::pkey::PKey;
use leyline_bssl::rsa::Rsa;
use leyline_bssl::ssl::{SslContextBuilder, SslMethod};
use leyline_bssl::x509::extension::{BasicConstraints, ExtendedKeyUsage, SubjectAlternativeName};
use leyline_bssl::x509::{X509, X509NameBuilder};
use leyline_quiche::test_utils::{Pipe, emit_flight, process_flight};
use sha2::{Digest, Sha256};

#[test]
fn quic_verifier_checks_host_ca_and_pin() {
    let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_text("CN", "quic.tech").unwrap();
    let name = name.build();
    let mut certificate = X509::builder().unwrap();
    certificate.set_version(2).unwrap();
    certificate
        .set_serial_number(&BigNum::from_u32(1).unwrap().to_asn1_integer().unwrap())
        .unwrap();
    certificate.set_subject_name(&name).unwrap();
    certificate.set_issuer_name(&name).unwrap();
    certificate.set_pubkey(&key).unwrap();
    certificate
        .set_not_before(&Asn1Time::days_from_now(0).unwrap())
        .unwrap();
    certificate
        .set_not_after(&Asn1Time::days_from_now(1).unwrap())
        .unwrap();
    certificate
        .append_extension(&BasicConstraints::new().critical().ca().build().unwrap())
        .unwrap();
    certificate
        .append_extension(&ExtendedKeyUsage::new().server_auth().build().unwrap())
        .unwrap();
    let san = SubjectAlternativeName::new()
        .dns("quic.tech")
        .build(&certificate.x509v3_context(None, None))
        .unwrap();
    certificate.append_extension(&san).unwrap();
    certificate.sign(&key, MessageDigest::sha256()).unwrap();
    let certificate = certificate.build();
    let der = certificate.to_der().unwrap();
    let pin: [u8; 32] = Sha256::digest(&der).into();
    let profile =
        BrowserProfile::from_toml(include_str!("../../../profiles/chrome/147.toml")).unwrap();

    for (host, anchor, pinned, accepted) in [
        ("quic.tech", true, Some(pin), true),
        ("quic.tech", true, None, true),
        ("wrong.example", true, Some(pin), false),
        ("quic.tech", true, Some([0; 32]), false),
        ("quic.tech", false, None, false),
    ] {
        let mut trust = TlsTrustConfig::new().env_roots(false);
        if anchor {
            trust = trust.add_ca_der(der.clone());
        }
        if let Some(pin) = pinned {
            trust = trust.add_pinned_leaf_sha256(pin);
        }
        let mut client = H3Config::from_profile(&profile)
            .unwrap()
            .build_quic_config(&trust, host)
            .unwrap();
        let mut server = SslContextBuilder::new(SslMethod::tls()).unwrap();
        server.set_certificate(&certificate).unwrap();
        server.set_private_key(&key).unwrap();
        let mut server = leyline_quiche::Config::with_boring_ssl_ctx_builder(
            leyline_quiche::PROTOCOL_VERSION,
            server,
        )
        .unwrap();
        server
            .set_application_protos(leyline_quiche::h3::APPLICATION_PROTOCOL)
            .unwrap();
        let mut pipe = Pipe::with_client_and_server_config(&mut client, &mut server).unwrap();
        for _ in 0..8 {
            let outcome = emit_flight(&mut pipe.client)
                .and_then(|flight| process_flight(&mut pipe.server, flight))
                .and_then(|()| emit_flight(&mut pipe.server))
                .and_then(|flight| process_flight(&mut pipe.client, flight));
            if outcome.is_err() || pipe.client.is_established() {
                break;
            }
        }
        assert_eq!(
            pipe.client.is_established(),
            accepted,
            "{host}, anchor={anchor}, pin={pinned:?}"
        );
    }
}

#[tokio::test]
async fn resolve_peer_prefers_ipv4_for_localhost() {
    let addr = resolve_peer(&crate::tls::SystemResolver, "localhost", 443)
        .await
        .expect("resolve localhost");
    assert!(addr.is_ipv4(), "got {addr}");
}

#[tokio::test]
async fn resolve_peer_falls_back_on_ipv6_only_hosts() {
    let addr = resolve_peer(&crate::tls::SystemResolver, "::1", 443)
        .await
        .expect("resolve ::1");
    assert!(addr.is_ipv6(), "got {addr}");
}

#[test]
fn validates_profile_connection_id_lengths() {
    validate_connection_id_len(8).expect("expected Ok");
    assert!(validate_connection_id_len(0).is_err());
    assert!(validate_connection_id_len(leyline_quiche::MAX_CONN_ID_LEN + 1).is_err());
}

#[test]
fn body_budget_allows_zero_chunks() {
    check_body_budget(0, 0, 1024).expect("expected Ok");
    check_body_budget(1024, 0, 1024).expect("expected Ok");
}

#[test]
fn body_budget_allows_exactly_max() {
    check_body_budget(0, 1024, 1024).expect("expected Ok");
    check_body_budget(512, 512, 1024).expect("expected Ok");
}

#[test]
fn body_budget_rejects_past_max_by_one_byte() {
    let err = check_body_budget(1024, 1, 1024).unwrap_err();
    assert_eq!(err, 1025);
}

#[test]
fn body_budget_rejects_large_chunk_past_cap() {
    let err = check_body_budget(0, 100 * 1024 * 1024 + 1, 100 * 1024 * 1024).unwrap_err();
    assert_eq!(err, (100 * 1024 * 1024 + 1) as u64);
}

#[test]
fn body_budget_saturates_on_usize_add_overflow() {
    let err = check_body_budget(usize::MAX, 1, 100 * 1024 * 1024).unwrap_err();
    assert_eq!(err, usize::MAX as u64);
}
