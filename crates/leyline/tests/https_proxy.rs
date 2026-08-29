//! Integration test: `https://` CONNECT proxy (TLS to the proxy itself).
//!
//! For an `https://` proxy the client→proxy leg is itself TLS, so the CONNECT
//! request — including `Proxy-Authorization` credentials — travels *encrypted*,
//! and the real origin handshake nests inside that proxy TLS.
//!
//! This stands up an in-process mock proxy that:
//!   1. accepts a TLS handshake (the proxy leg),
//!   2. reads the CONNECT request off the decrypted stream (proving it arrived
//!      inside TLS, not in cleartext) and reports it back,
//!   3. answers `200 Connection established`,
//!   4. accepts a SECOND, nested TLS handshake (the origin leg) over the same
//!      stream.
//!
//! The production `FingerprintConnector` drives the whole chain. `connect`
//! returning `Ok` proves both handshakes completed and the origin cert
//! verified against the trusted CA through the tunnel; the captured CONNECT
//! proves the credentials were encrypted.
#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use std::net::SocketAddr;
use std::sync::Arc;

use leyline::TcpProfile;
use leyline::profile::BrowserProfile;
use leyline::tls::{FingerprintConnector, TlsTrustConfig};
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

/// Generate a CA and a leaf cert carrying every name in `dns_names` as a SAN,
/// so the one cert serves both the proxy leg (`localhost`) and the origin leg
/// (`right.example`).
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

/// Read from `s` until the `\r\n\r\n` header terminator; return the bytes as a
/// String.
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

/// Spawn a mock `https://` CONNECT proxy. Returns its address and a receiver
/// that yields the CONNECT request the proxy decrypted off the TLS stream.
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

        // Proxy leg: TLS to the proxy itself.
        let ssl = Ssl::new(acceptor.context()).unwrap();
        let mut proxy_tls = leyline_bssl_tokio::SslStream::new(ssl, tcp).unwrap();
        std::pin::Pin::new(&mut proxy_tls).accept().await.unwrap();

        // The CONNECT request arrives decrypted here — i.e. it was encrypted on
        // the wire. Report it so the test can assert the credentials.
        let connect_req = read_head(&mut proxy_tls).await;
        let _ = tx.send(connect_req);

        proxy_tls
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .await
            .unwrap();
        proxy_tls.flush().await.unwrap();

        // Origin leg: a SECOND, nested TLS handshake over the proxy TLS — the
        // real fingerprinted handshake the client performs to the target.
        let ssl2 = Ssl::new(acceptor.context()).unwrap();
        let mut origin_tls = leyline_bssl_tokio::SslStream::new(ssl2, proxy_tls).unwrap();
        let _ = std::pin::Pin::new(&mut origin_tls).accept().await;
        // Client returns its handshaked stream without sending an app request,
        // so nothing more to read; the task ends.
    });

    (addr, rx)
}

fn connector(r#gen: &Generated) -> FingerprintConnector {
    // Trust only the test CA — no env/system roots — so a successful origin
    // handshake through the tunnel proves real verification, not a bypass.
    let trust = TlsTrustConfig::new()
        .without_env_roots()
        .without_system_roots()
        .add_ca_der(r#gen.ca_der.clone());
    FingerprintConnector::new_with_trust(&load_profile(), TcpProfile::LINUX, &trust)
        .expect("connector build")
}

#[tokio::test]
async fn https_proxy_tunnels_with_encrypted_connect() {
    // One cert covers both legs: `localhost` (the proxy, reached via the system
    // resolver) and `right.example` (the CONNECT target / nested SNI).
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

    // The proxy decrypted the CONNECT off its TLS stream — so it was encrypted
    // on the wire — and it carried the Basic credentials (`user:secret`).
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
    // A connector carrying an origin-specific TLS identity (here, leaf pins)
    // must REFUSE an https:// proxy rather than present the origin identity to
    // the proxy or check the proxy's cert against the origin's pins. The guard
    // fires before any socket is opened, so the unroutable :1 is never dialed.
    let r#gen = generate_chain(&["right.example"]);
    let trust = TlsTrustConfig::new()
        .without_env_roots()
        .without_system_roots()
        .add_ca_der(r#gen.ca_der.clone())
        .add_pinned_leaf_sha256([0u8; 32]);
    let conn = FingerprintConnector::new_with_trust(&load_profile(), TcpProfile::LINUX, &trust)
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
