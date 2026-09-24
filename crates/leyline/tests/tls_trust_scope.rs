#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use std::net::SocketAddr;
use std::sync::Arc;

use leyline::Platform;
use leyline::profile::BrowserProfile;
use leyline::tls::{FingerprintConnector, ResolveFuture, Resolver, TlsTrustConfig};
use leyline_bssl::asn1::Asn1Time;
use leyline_bssl::bn::{BigNum, MsbOption};
use leyline_bssl::hash::MessageDigest;
use leyline_bssl::pkey::PKey;
use leyline_bssl::rsa::Rsa;
use leyline_bssl::x509::extension::{BasicConstraints, SubjectAlternativeName};
use leyline_bssl::x509::{X509, X509NameBuilder};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

struct LoopbackResolver(SocketAddr);

impl Resolver for LoopbackResolver {
    fn resolve<'a>(&'a self, _host: &'a str, _port: u16) -> ResolveFuture<'a> {
        let addr = self.0;
        Box::pin(async move { Ok(vec![addr]) })
    }
}

struct Chain {
    ca_cert_pem: Vec<u8>,
    leaf_cert_pem: Vec<u8>,
    leaf_key_pem: Vec<u8>,
}

fn generate_chain(leaf_dns: &str) -> Chain {
    let ca_key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_text("CN", "leyline-trust-scope-ca")
        .unwrap();
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

    Chain {
        ca_cert_pem: ca.to_pem().unwrap(),
        leaf_cert_pem: leaf.to_pem().unwrap(),
        leaf_key_pem: leaf_key.private_key_to_pem_pkcs8().unwrap(),
    }
}

fn load_profile() -> BrowserProfile {
    BrowserProfile::from_toml(include_str!("../profiles/chrome/147.toml"))
        .expect("chrome 147 profile parses")
}

async fn spawn_tls_server(chain: &Chain) -> SocketAddr {
    use leyline_bssl::ssl::{SslAcceptor, SslMethod};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let leaf_pem = chain.leaf_cert_pem.clone();
    let key_pem = chain.leaf_key_pem.clone();
    let ca_pem = chain.ca_cert_pem.clone();

    tokio::spawn(async move {
        let key = PKey::private_key_from_pem(&key_pem).unwrap();
        let leaf = X509::from_pem(&leaf_pem).unwrap();
        let ca = X509::from_pem(&ca_pem).unwrap();
        let mut builder = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls()).unwrap();
        builder.set_private_key(&key).unwrap();
        builder.set_certificate(&leaf).unwrap();
        builder.add_extra_chain_cert(ca).unwrap();
        let acceptor = builder.build();

        for _ in 0..3 {
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

#[tokio::test]
async fn without_env_roots_excludes_environment_roots() {
    static ENV_LOCK: Mutex<()> = Mutex::const_new(());
    let _env_lock = ENV_LOCK.lock().await;
    let chain = generate_chain("trust.example");

    let ca_path =
        std::env::temp_dir().join(format!("leyline-trust-scope-ca-{}.pem", std::process::id()));
    std::fs::write(&ca_path, &chain.ca_cert_pem).unwrap();
    let old_ca = std::env::var_os("SSL_CERT_FILE");
    // SAFETY: `ENV_LOCK` serializes this test's process-global environment mutation, and this binary has no other tests that read `SSL_CERT_FILE`.
    unsafe {
        std::env::set_var("SSL_CERT_FILE", &ca_path);
    }

    let addr = spawn_tls_server(&chain).await;

    let control = TlsTrustConfig::new();
    let res = FingerprintConnector::new_with_trust(
        &load_profile(),
        Platform::Linux.tcp_profile(),
        &control,
    )
    .expect("connector build")
    .with_resolver(Arc::new(LoopbackResolver(addr)))
    .connect("trust.example", 443, None)
    .await;
    assert!(
        res.is_ok(),
        "default trust must honour SSL_CERT_FILE (err: {:?})",
        res.err().map(|e| e.to_string())
    );

    let restricted = TlsTrustConfig::new().without_env_roots();
    let res = FingerprintConnector::new_with_trust(
        &load_profile(),
        Platform::Linux.tcp_profile(),
        &restricted,
    )
    .expect("connector build")
    .with_resolver(Arc::new(LoopbackResolver(addr)))
    .connect("trust.example", 443, None)
    .await;
    assert!(
        res.is_err(),
        "without_env_roots() must not trust a CA that only SSL_CERT_FILE provides"
    );

    let explicit = TlsTrustConfig::new()
        .without_env_roots()
        .add_ca_file(&ca_path);
    let res = FingerprintConnector::new_with_trust(
        &load_profile(),
        Platform::Linux.tcp_profile(),
        &explicit,
    )
    .expect("connector build")
    .with_resolver(Arc::new(LoopbackResolver(addr)))
    .connect("trust.example", 443, None)
    .await;
    assert!(
        res.is_ok(),
        "explicit private root must be trusted: {:?}",
        res.err().map(|e| e.to_string())
    );

    // SAFETY: the same lock still excludes concurrent environment readers; restore the process state before releasing it.
    unsafe {
        match old_ca {
            Some(value) => std::env::set_var("SSL_CERT_FILE", value),
            None => std::env::remove_var("SSL_CERT_FILE"),
        }
    }
    let _ = std::fs::remove_file(&ca_path);
}
