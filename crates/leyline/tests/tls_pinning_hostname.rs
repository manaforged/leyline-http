#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use std::net::SocketAddr;
use std::sync::Arc;

use leyline::TcpProfile;
use leyline::profile::BrowserProfile;
use leyline::tls::{FingerprintConnector, ResolveFuture, Resolver, TlsError, TlsTrustConfig};
use leyline_bssl::asn1::Asn1Time;
use leyline_bssl::bn::{BigNum, MsbOption};
use leyline_bssl::hash::MessageDigest;
use leyline_bssl::pkey::PKey;
use leyline_bssl::rsa::Rsa;
use leyline_bssl::x509::extension::{BasicConstraints, SubjectAlternativeName};
use leyline_bssl::x509::{X509, X509NameBuilder};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

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

#[derive(Clone, Copy)]
enum San<'a> {
    Dns(&'a str),
    Ip(&'a str),
}

fn generate_chain(leaf_cn: &str, leaf_san: San) -> Generated {
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

    let leaf_key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let mut leaf_name = X509NameBuilder::new().unwrap();
    leaf_name.append_entry_by_text("CN", leaf_cn).unwrap();
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
    let mut san_builder = SubjectAlternativeName::new();
    match leaf_san {
        San::Dns(d) => san_builder.dns(d),
        San::Ip(ip) => san_builder.ip(ip),
    };
    let san = san_builder
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

async fn spawn_tls_server(r#gen: &Generated) -> SocketAddr {
    use leyline_bssl::ssl::{SslAcceptor, SslMethod};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let leaf_pem = r#gen.leaf_cert_pem.clone();
    let key_pem = r#gen.leaf_key_pem.clone();
    let ca_pem = r#gen.ca_cert_pem.clone();

    tokio::spawn(async move {
        let key = PKey::private_key_from_pem(&key_pem).unwrap();
        let leaf = X509::from_pem(&leaf_pem).unwrap();
        let ca = X509::from_pem(&ca_pem).unwrap();
        let mut builder = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls()).unwrap();
        builder.set_private_key(&key).unwrap();
        builder.set_certificate(&leaf).unwrap();
        builder.add_extra_chain_cert(ca).unwrap();
        let acceptor = builder.build();

        for _ in 0..2 {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            let ssl = leyline_bssl::ssl::Ssl::new(acceptor.context()).unwrap();
            let _ = leyline_bssl_tokio::SslStreamBuilder::new(ssl, tcp)
                .accept()
                .await;
        }
    });

    addr
}

fn connector(r#gen: &Generated, addr: SocketAddr) -> FingerprintConnector {
    let trust = TlsTrustConfig::new()
        .without_env_roots()
        .without_system_roots()
        .add_ca_der(r#gen.ca_der.clone())
        .add_pinned_leaf_sha256(r#gen.leaf_pin);
    FingerprintConnector::new_with_trust(&load_profile(), TcpProfile::LINUX, &trust)
        .expect("connector build")
        .with_resolver(Arc::new(LoopbackResolver(addr)))
}

fn connector_with_trust(trust: TlsTrustConfig, addr: SocketAddr) -> FingerprintConnector {
    FingerprintConnector::new_with_trust(&load_profile(), TcpProfile::LINUX, &trust)
        .expect("connector build")
        .with_resolver(Arc::new(LoopbackResolver(addr)))
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn native_system_trust_preserves_custom_ca_hostname_and_pins() {
    let generated = generate_chain("native.example", San::Dns("native.example"));
    for (host, ca, pin, expected) in [
        ("native.example", true, Some(generated.leaf_pin), "ok"),
        ("native.example", true, None, "ok"),
        ("other.example", true, Some(generated.leaf_pin), "hostname"),
        ("native.example", true, Some([0; 32]), "pin"),
        ("native.example", false, None, "certificate"),
    ] {
        let mut trust = TlsTrustConfig::new().without_env_roots();
        if ca {
            trust = trust.add_ca_der(generated.ca_der.clone());
        }
        if let Some(pin) = pin {
            trust = trust.add_pinned_leaf_sha256(pin);
        }
        let addr = spawn_tls_server(&generated).await;
        let result = connector_with_trust(trust, addr)
            .connect(host, 443, None)
            .await;
        match expected {
            "ok" => assert!(result.is_ok(), "{host}: {:?}", result.err()),
            "hostname" => assert!(matches!(result, Err(TlsError::Hostname(_)))),
            "pin" => assert!(matches!(result, Err(TlsError::Pinning(_)))),
            "certificate" => assert!(matches!(result, Err(TlsError::Certificate(_)))),
            _ => unreachable!(),
        }
    }
}

#[tokio::test]
async fn pinned_cert_still_accepts_matching_hostname() {
    let r#gen = generate_chain("wrong.example", San::Dns("wrong.example"));
    let _ = r#gen.leaf_der;
    let addr = spawn_tls_server(&r#gen).await;
    let res = connector(&r#gen, addr)
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
    let r#gen = generate_chain("wrong.example", San::Dns("wrong.example"));
    let addr = spawn_tls_server(&r#gen).await;
    let res = connector(&r#gen, addr)
        .connect("right.example", 443, None)
        .await;
    let err = res.err().expect("hostname mismatch must fail");
    assert!(matches!(err, TlsError::Hostname(_)), "got {err:?}");
}

#[tokio::test]
async fn default_verifier_reports_hostname_mismatch() {
    let r#gen = generate_chain("wrong.example", San::Dns("wrong.example"));
    let addr = spawn_tls_server(&r#gen).await;
    let trust = TlsTrustConfig::new()
        .without_env_roots()
        .without_system_roots()
        .add_ca_der(r#gen.ca_der.clone());

    let err = connector_with_trust(trust, addr)
        .connect("right.example", 443, None)
        .await
        .err()
        .expect("hostname mismatch must fail");
    assert!(matches!(err, TlsError::Hostname(_)), "got {err:?}");
}

#[tokio::test]
async fn pinned_cert_accepts_matching_ip_san() {
    let r#gen = generate_chain("127.0.0.1", San::Ip("127.0.0.1"));
    let addr = spawn_tls_server(&r#gen).await;
    let res = connector(&r#gen, addr)
        .connect("127.0.0.1", 443, None)
        .await;
    assert!(
        res.is_ok(),
        "matching-IP pinned handshake must succeed, got err: {:?}",
        res.err().map(|e| e.to_string())
    );
}

#[tokio::test]
async fn pinned_cert_rejects_mismatched_ip() {
    let r#gen = generate_chain("127.0.0.1", San::Ip("127.0.0.1"));
    let addr = spawn_tls_server(&r#gen).await;
    let res = connector(&r#gen, addr)
        .connect("127.0.0.2", 443, None)
        .await;
    let err = res.err().expect("IP mismatch must fail");
    assert!(matches!(err, TlsError::Hostname(_)), "got {err:?}");
}

#[tokio::test]
async fn certificate_and_pinning_failures_are_permanent() {
    let r#gen = generate_chain("right.example", San::Dns("right.example"));

    let addr = spawn_tls_server(&r#gen).await;
    let trust = TlsTrustConfig::new()
        .without_env_roots()
        .without_system_roots();
    let err = connector_with_trust(trust, addr)
        .connect("right.example", 443, None)
        .await
        .err()
        .expect("untrusted chain must fail");
    assert!(matches!(err, TlsError::Certificate(_)), "got {err:?}");

    let addr = spawn_tls_server(&r#gen).await;
    let trust = TlsTrustConfig::new()
        .without_env_roots()
        .without_system_roots()
        .add_ca_der(r#gen.ca_der.clone())
        .add_pinned_leaf_sha256([0; 32]);
    let err = connector_with_trust(trust, addr)
        .connect("right.example", 443, None)
        .await
        .err()
        .expect("wrong pin must fail");
    assert!(matches!(err, TlsError::Pinning(_)), "got {err:?}");
}

#[tokio::test]
async fn tcp_connect_failure_is_retryable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let trust = TlsTrustConfig::new()
        .without_env_roots()
        .without_system_roots();
    let err = connector_with_trust(trust, addr)
        .connect("offline.example", 443, None)
        .await
        .err()
        .expect("closed listener must reject the TCP connection");
    assert!(matches!(err, TlsError::TcpConnect(_)), "got {err:?}");
}

#[tokio::test]
async fn handshake_transport_failure_is_retryable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        drop(tcp);
    });

    let trust = TlsTrustConfig::new()
        .without_env_roots()
        .without_system_roots();
    let err = connector_with_trust(trust, addr)
        .connect("closed.example", 443, None)
        .await
        .err()
        .expect("peer closing during the handshake must fail");
    assert!(matches!(err, TlsError::HandshakeIo(_)), "got {err:?}");
}

#[tokio::test]
async fn handshake_protocol_failure_is_retryable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (mut tcp, _) = listener.accept().await.unwrap();
        tcp.write_all(b"this is not TLS")
            .await
            .expect("send non-TLS bytes");
        tcp
    });

    let trust = TlsTrustConfig::new()
        .without_env_roots()
        .without_system_roots();
    let err = connector_with_trust(trust, addr)
        .connect("plaintext.example", 443, None)
        .await
        .err()
        .expect("a non-TLS peer must fail the handshake");
    drop(peer.await.expect("peer task completes"));
    assert!(matches!(err, TlsError::Handshake(_)), "got {err:?}");
}
