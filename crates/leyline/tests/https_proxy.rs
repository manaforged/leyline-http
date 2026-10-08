#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use std::net::SocketAddr;
use std::sync::Arc;

use leyline::tls::FingerprintConnector;
use leyline::{BrowserProfile, Platform, TlsTrustConfig};
use leyline_bssl::asn1::Asn1Time;
use leyline_bssl::bn::{BigNum, MsbOption};
use leyline_bssl::hash::MessageDigest;
use leyline_bssl::pkey::PKey;
use leyline_bssl::rsa::Rsa;
use leyline_bssl::x509::extension::{BasicConstraints, SubjectAlternativeName};
use leyline_bssl::x509::{X509, X509NameBuilder};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

struct Generated {
    ca_der: Vec<u8>,
    leaf_key_pem: Vec<u8>,
    leaf_cert_pem: Vec<u8>,
    ca_cert_pem: Vec<u8>,
}

fn generate_chain(dns_names: &[&str]) -> Generated {
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
    leaf_name.append_entry_by_text("CN", dns_names[0]).unwrap();
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
    let mut san = SubjectAlternativeName::new();
    for dns in dns_names {
        san.dns(dns);
    }
    let san = san.build(&leaf.x509v3_context(Some(&ca), None)).unwrap();
    leaf.append_extension(&san).unwrap();
    leaf.sign(&ca_key, MessageDigest::sha256()).unwrap();
    let leaf = leaf.build();

    Generated {
        ca_der: ca.to_der().unwrap(),
        leaf_key_pem: leaf_key.private_key_to_pem_pkcs8().unwrap(),
        leaf_cert_pem: leaf.to_pem().unwrap(),
        ca_cert_pem: ca.to_pem().unwrap(),
    }
}

fn load_profile() -> BrowserProfile {
    BrowserProfile::from_toml(include_str!("../profiles/chrome/147.toml"))
        .expect("chrome 147 profile parses")
}

async fn read_head<S>(s: &mut S) -> String
where
    S: AsyncReadExt + Unpin,
{
    let mut buf = Vec::new();
    let mut tmp = [0u8; 256];
    loop {
        let n = s.read(&mut tmp).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

async fn spawn_mock_https_proxy(r#gen: &Generated) -> (SocketAddr, oneshot::Receiver<String>) {
    use leyline_bssl::ssl::{Ssl, SslAcceptor, SslMethod};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let leaf_pem = r#gen.leaf_cert_pem.clone();
    let key_pem = r#gen.leaf_key_pem.clone();
    let ca_pem = r#gen.ca_cert_pem.clone();
    let (tx, rx) = oneshot::channel::<String>();

    tokio::spawn(async move {
        let key = PKey::private_key_from_pem(&key_pem).unwrap();
        let leaf = X509::from_pem(&leaf_pem).unwrap();
        let ca = X509::from_pem(&ca_pem).unwrap();
        let mut builder = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls()).unwrap();
        builder.set_private_key(&key).unwrap();
        builder.set_certificate(&leaf).unwrap();
        builder.add_extra_chain_cert(ca).unwrap();
        let acceptor = builder.build();

        let (tcp, _) = listener.accept().await.unwrap();

        let ssl = Ssl::new(acceptor.context()).unwrap();
        let mut proxy_tls = leyline_bssl_tokio::SslStreamBuilder::new(ssl, tcp)
            .accept()
            .await
            .unwrap();

        let connect_req = read_head(&mut proxy_tls).await;
        drop(tx.send(connect_req));

        proxy_tls
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .await
            .unwrap();
        proxy_tls.flush().await.unwrap();

        let ssl2 = Ssl::new(acceptor.context()).unwrap();
        drop(
            leyline_bssl_tokio::SslStreamBuilder::new(ssl2, proxy_tls)
                .accept()
                .await,
        );
    });

    (addr, rx)
}

fn connector(r#gen: &Generated) -> FingerprintConnector {
    let trust = TlsTrustConfig::new()
        .env_roots(false)
        .system_roots(false)
        .add_ca_der(r#gen.ca_der.clone());
    FingerprintConnector::new_with_trust(&load_profile(), Platform::Linux.tcp_profile(), &trust)
        .expect("connector build")
}

#[tokio::test]
async fn https_proxy_tunnels_with_encrypted_connect() {
    let r#gen = generate_chain(&["localhost", "right.example"]);
    let (addr, connect_rx) = spawn_mock_https_proxy(&r#gen).await;

    let proxy_url = format!("https://user:secret@localhost:{}", addr.port());
    let res = Arc::new(connector(&r#gen))
        .connect("right.example", 443, Some(&proxy_url))
        .await;
    assert!(
        res.is_ok(),
        "https-proxy nested handshake must succeed (TLS to proxy + CONNECT + \
         nested TLS to origin, all CA-verified), got err: {:?}",
        res.err().map(|e| e.to_string())
    );

    let connect_req = connect_rx.await.expect("proxy reported its CONNECT");
    assert!(
        connect_req.starts_with("CONNECT right.example:443 "),
        "proxy must receive a CONNECT to the target, got: {connect_req:?}"
    );
    assert!(
        connect_req.contains("Proxy-Authorization: Basic dXNlcjpzZWNyZXQ="),
        "credentials (user:secret) must travel inside the proxy TLS, got: {connect_req:?}"
    );
}

#[tokio::test]
async fn https_proxy_refused_when_connector_has_origin_identity() {
    let r#gen = generate_chain(&["right.example"]);
    let trust = TlsTrustConfig::new()
        .env_roots(false)
        .system_roots(false)
        .add_ca_der(r#gen.ca_der.clone())
        .add_pinned_leaf_sha256([0u8; 32]);
    let conn = FingerprintConnector::new_with_trust(
        &load_profile(),
        Platform::Linux.tcp_profile(),
        &trust,
    )
    .expect("connector build");

    let err = match conn
        .connect("right.example", 443, Some("https://localhost:1"))
        .await
    {
        Ok(_) => panic!("https proxy + origin pins must be refused"),
        Err(e) => e,
    };
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("client certificate") || msg.contains("pin"),
        "expected an origin-identity refusal (no leak to the proxy), got: {msg}"
    );
}
