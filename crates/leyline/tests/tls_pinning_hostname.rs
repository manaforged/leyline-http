//! Integration test: certificate pinning must NOT bypass hostname
//! verification.
//!
//! leyline replaces BoringSSL's built-in verifier with a custom
//! callback when leaf pins are configured. `SSL_CTX_set_custom_verify`
//! replaces the *entire* verification path, including the
//! `X509_check_host` SAN match the built-in verifier performs. Without
//! an explicit hostname check, a CA-trusted, correctly-pinned
//! certificate issued for `wrong.example` would be accepted when
//! connecting to `right.example`.
//!
//! This test generates a private CA, issues a leaf for `wrong.example`,
//! stands up a real BoringSSL acceptor presenting it, and drives the
//! production `FingerprintConnector` against it with that leaf pinned
//! and the CA trusted:
//!
//! - control: connecting to `wrong.example` (the cert's SAN) succeeds —
//!   proving the chain + pin would otherwise accept the cert.
//! - regression: connecting to `right.example` fails — the hostname
//!   mismatch is caught even though chain + pin pass.

use std::net::SocketAddr;
use std::sync::Arc;

use btls::asn1::Asn1Time;
use btls::bn::{BigNum, MsbOption};
use btls::hash::MessageDigest;
use btls::pkey::PKey;
use btls::rsa::Rsa;
use btls::x509::extension::{BasicConstraints, SubjectAlternativeName};
use btls::x509::{X509NameBuilder, X509};
use leyline::profile::BrowserProfile;
use leyline::tcp::TcpProfile;
use leyline::tls::{FingerprintConnector, ResolveFuture, Resolver, TlsTrustConfig};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;

/// Resolver that maps every host to one fixed loopback address, so the
/// test can connect to an arbitrary SNI name against a local listener.
struct LoopbackResolver(SocketAddr);

impl Resolver for LoopbackResolver {
    fn resolve<'a>(&'a self, _host: &'a str, _port: u16) -> ResolveFuture<'a> {
        let addr = self.0;
        Box::pin(async move { Ok(vec![addr]) })
    }
}

struct Generated {
    ca_der: Vec<u8>,
    leaf_der: Vec<u8>,
    leaf_pin: [u8; 32],
    leaf_key_pem: Vec<u8>,
    leaf_cert_pem: Vec<u8>,
    ca_cert_pem: Vec<u8>,
}

/// Generate a CA and a leaf cert whose only SAN is `leaf_dns`.
fn generate_chain(leaf_dns: &str) -> Generated {
    // --- CA ---
    let ca_key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_text("CN", "leyline-test-ca").unwrap();
    let ca_name = name.build();

    let mut ca = X509::builder().unwrap();
    ca.set_version(2).unwrap();
    let serial = {
        let mut bn = BigNum::new().unwrap();
        bn.rand(159, MsbOption::MAYBE_ZERO, false).unwrap();
        bn.to_asn1_integer().unwrap()
    };
    ca.set_serial_number(&serial).unwrap();
    ca.set_subject_name(&ca_name).unwrap();
    ca.set_issuer_name(&ca_name).unwrap();
    ca.set_pubkey(&ca_key).unwrap();
    ca.set_not_before(&Asn1Time::days_from_now(0).unwrap())
        .unwrap();
    ca.set_not_after(&Asn1Time::days_from_now(1).unwrap())
        .unwrap();
    ca.append_extension(&BasicConstraints::new().critical().ca().build().unwrap())
        .unwrap();
    ca.sign(&ca_key, MessageDigest::sha256()).unwrap();
    let ca = ca.build();

    // --- leaf, signed by the CA ---
    let leaf_key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let mut leaf_name = X509NameBuilder::new().unwrap();
    leaf_name.append_entry_by_text("CN", leaf_dns).unwrap();
    let leaf_name = leaf_name.build();

    let mut leaf = X509::builder().unwrap();
    leaf.set_version(2).unwrap();
    let serial = {
        let mut bn = BigNum::new().unwrap();
        bn.rand(159, MsbOption::MAYBE_ZERO, false).unwrap();
        bn.to_asn1_integer().unwrap()
    };
    leaf.set_serial_number(&serial).unwrap();
    leaf.set_subject_name(&leaf_name).unwrap();
    leaf.set_issuer_name(ca.subject_name()).unwrap();
    leaf.set_pubkey(&leaf_key).unwrap();
    leaf.set_not_before(&Asn1Time::days_from_now(0).unwrap())
        .unwrap();
    leaf.set_not_after(&Asn1Time::days_from_now(1).unwrap())
        .unwrap();
    let san = SubjectAlternativeName::new()
        .dns(leaf_dns)
        .build(&leaf.x509v3_context(Some(&ca), None))
        .unwrap();
    leaf.append_extension(&san).unwrap();
    leaf.sign(&ca_key, MessageDigest::sha256()).unwrap();
    let leaf = leaf.build();

    let leaf_der = leaf.to_der().unwrap();
    let leaf_pin: [u8; 32] = Sha256::digest(&leaf_der).into();

    Generated {
        ca_der: ca.to_der().unwrap(),
        leaf_der: leaf_der.clone(),
        leaf_pin,
        leaf_key_pem: leaf_key.private_key_to_pem_pkcs8().unwrap(),
        leaf_cert_pem: leaf.to_pem().unwrap(),
        ca_cert_pem: ca.to_pem().unwrap(),
    }
}

fn load_profile() -> BrowserProfile {
    BrowserProfile::from_toml(include_str!("../profiles/chrome/147.toml"))
        .expect("chrome 147 profile parses")
}

/// Spawn a one-shot BoringSSL acceptor presenting `leaf` + `ca`. Returns
/// the bound address; the task serves a single handshake then exits.
async fn spawn_tls_server(gen: &Generated) -> SocketAddr {
    use btls::ssl::{SslAcceptor, SslMethod};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let leaf_pem = gen.leaf_cert_pem.clone();
    let key_pem = gen.leaf_key_pem.clone();
    let ca_pem = gen.ca_cert_pem.clone();

    tokio::spawn(async move {
        let key = PKey::private_key_from_pem(&key_pem).unwrap();
        let leaf = X509::from_pem(&leaf_pem).unwrap();
        let ca = X509::from_pem(&ca_pem).unwrap();
        let mut builder = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls()).unwrap();
        builder.set_private_key(&key).unwrap();
        builder.set_certificate(&leaf).unwrap();
        builder.add_extra_chain_cert(ca).unwrap();
        let acceptor = builder.build();

        // Serve a couple of handshakes (control + regression attempts).
        for _ in 0..2 {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            let ssl = btls::ssl::Ssl::new(acceptor.context()).unwrap();
            let mut stream = match tokio_btls::SslStream::new(ssl, tcp) {
                Ok(s) => s,
                Err(_) => continue,
            };
            // A client that aborts on hostname mismatch makes accept()
            // fail; that's expected for the regression case.
            let _ = std::pin::Pin::new(&mut stream).accept().await;
        }
    });

    addr
}

fn connector(gen: &Generated, addr: SocketAddr) -> FingerprintConnector {
    let trust = TlsTrustConfig::new()
        .without_env_roots()
        .without_system_roots()
        .add_ca_der(gen.ca_der.clone())
        .add_pinned_leaf_sha256(gen.leaf_pin);
    FingerprintConnector::new_with_trust(&load_profile(), TcpProfile::LINUX, &trust)
        .expect("connector build")
        .with_resolver(Arc::new(LoopbackResolver(addr)))
}

#[tokio::test]
async fn pinned_cert_still_accepts_matching_hostname() {
    let gen = generate_chain("wrong.example");
    let _ = gen.leaf_der;
    let addr = spawn_tls_server(&gen).await;
    // Control: connect to the cert's actual SAN — chain + pin + host
    // all match, so the handshake must succeed.
    let res = connector(&gen, addr)
        .connect("wrong.example", 443, None)
        .await;
    assert!(
        res.is_ok(),
        "matching-hostname pinned handshake must succeed, got err: {:?}",
        res.err().map(|e| e.to_string())
    );
}

#[tokio::test]
async fn pinned_cert_rejects_mismatched_hostname() {
    let gen = generate_chain("wrong.example");
    let addr = spawn_tls_server(&gen).await;
    // Regression: the cert's SAN is wrong.example, but we connect to
    // right.example. Chain verifies and the pin matches, yet the
    // hostname does not — the handshake MUST fail.
    let res = connector(&gen, addr)
        .connect("right.example", 443, None)
        .await;
    assert!(
        res.is_err(),
        "pinned cert for wrong.example must be rejected when connecting to right.example"
    );
}
