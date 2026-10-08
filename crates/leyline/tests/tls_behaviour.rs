#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
#[path = "it/tls_support/mod.rs"]
mod tls_support;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use leyline::tls::{ResolveFuture, Resolver};
use leyline::{ProtocolPolicy, Session, TlsTrustConfig};
use leyline_bssl::hash::{MessageDigest, hash};
use leyline_bssl_tokio::SslStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn reply_ok(mut stream: SslStream<TcpStream>) {
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(read) => head.extend_from_slice(&buf[..read]),
        }
    }
    drop(
        stream
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
            .await,
    );
}

async fn self_signed_server() -> (u16, Vec<u8>) {
    let (cert, key) = tls_support::self_signed();
    let der = cert.to_der().unwrap();
    let port = tls_support::tls_server_per_connection(
        cert,
        key,
        Arc::new(AtomicUsize::new(0)),
        |_| tls_support::HTTP11,
        |_, stream| reply_ok(stream),
    )
    .await;
    (port, der)
}

fn bare_trust() -> TlsTrustConfig {
    TlsTrustConfig::new().env_roots(false).system_roots(false)
}

async fn fetch(trust: TlsTrustConfig, url: String) -> leyline::Result<String> {
    Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .tls_trust(trust)
        .build()
        .unwrap()
        .get(url)
        .send()
        .await?
        .text()
        .await
}

#[tokio::test]
async fn a_self_signed_server_is_refused_without_trust_and_reached_with_danger_accept() {
    let (port, _) = self_signed_server().await;
    let url = format!("https://127.0.0.1:{port}/");

    fetch(bare_trust(), url.clone())
        .await
        .expect_err("expected Err");
    assert_eq!(
        fetch(bare_trust().danger_accept_invalid_certs(true), url)
            .await
            .unwrap(),
        "ok"
    );
}

#[tokio::test]
async fn a_pinned_leaf_connects_only_when_the_pin_matches() {
    let (port, der) = self_signed_server().await;
    let url = format!("https://127.0.0.1:{port}/");
    let digest = hash(MessageDigest::sha256(), &der).unwrap();
    let mut pin = [0u8; 32];
    pin.copy_from_slice(&digest);

    assert_eq!(
        fetch(
            bare_trust()
                .add_ca_der(der.clone())
                .add_pinned_leaf_sha256(pin),
            url.clone()
        )
        .await
        .unwrap(),
        "ok"
    );
    fetch(
        bare_trust().add_ca_der(der).add_pinned_leaf_sha256([7; 32]),
        url,
    )
    .await
    .expect_err("expected Err");
}

struct Recording {
    target: SocketAddr,
    asked: Mutex<Vec<(String, u16)>>,
}

impl Resolver for Recording {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a> {
        self.asked.lock().unwrap().push((host.to_owned(), port));
        let target = self.target;
        Box::pin(async move { Ok(vec![target]) })
    }
}

#[tokio::test]
async fn a_custom_resolver_decides_where_a_hostname_connects() {
    let (port, der) = self_signed_server().await;
    let resolver = Arc::new(Recording {
        target: SocketAddr::from(([127, 0, 0, 1], port)),
        asked: Mutex::new(Vec::new()),
    });

    let body = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .tls_trust(
            bare_trust()
                .add_ca_der(der)
                .danger_accept_invalid_certs(true),
        )
        .dns(Arc::clone(&resolver) as Arc<dyn Resolver>)
        .build()
        .unwrap()
        .get(format!("https://shop.invalid:{port}/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    assert_eq!(body, "ok");
    assert_eq!(
        resolver.asked.lock().unwrap().as_slice(),
        &[("shop.invalid".to_owned(), port)]
    );
}

fn http1_session(trust: TlsTrustConfig) -> Session {
    Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .tls_trust(trust)
        .build()
        .unwrap()
}

struct ResumingServer {
    url: String,
    der: Vec<u8>,
    pem: Vec<u8>,
    resumed: Arc<AtomicUsize>,
}

async fn resuming_server() -> ResumingServer {
    let (cert, key) = tls_support::self_signed();
    let der = cert.to_der().unwrap();
    let pem = cert.to_pem().unwrap();
    let resumed = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&resumed);
    let port = tls_support::tls_server_per_connection(
        cert,
        key,
        Arc::new(AtomicUsize::new(0)),
        |_| tls_support::HTTP11,
        move |_, stream: SslStream<TcpStream>| {
            let counter = Arc::clone(&counter);
            async move {
                if stream.ssl().session_reused() {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
                reply_ok(stream).await;
            }
        },
    )
    .await;
    ResumingServer {
        url: format!("https://127.0.0.1:{port}/"),
        der,
        pem,
        resumed,
    }
}

fn scratch_file(name: &str, contents: &[u8]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("leyline-trust-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    path
}

fn unrelated_pem() -> Vec<u8> {
    tls_support::self_signed().0.to_pem().unwrap()
}

#[tokio::test]
async fn a_saved_tls_session_resumes_only_under_the_trust_that_made_it() {
    let server = resuming_server().await;
    let unpinned = http1_session(bare_trust().add_ca_der(server.der.clone()));
    let first = unpinned.get(server.url.clone()).send().await.unwrap();
    assert_eq!(first.text().await.unwrap(), "ok");
    let state = unpinned.state();

    let pinned = http1_session(
        bare_trust()
            .add_ca_der(server.der.clone())
            .add_pinned_leaf_sha256([7; 32]),
    );
    state.restore_into(&pinned);
    let refused = pinned.get(server.url.clone()).send().await.unwrap_err();
    assert!(refused.tls().is_some(), "{refused:?}");
    assert_eq!(server.resumed.load(Ordering::SeqCst), 0);

    let same = http1_session(bare_trust().add_ca_der(server.der));
    state.restore_into(&same);
    let again = same.get(server.url).send().await.unwrap();
    assert_eq!(again.text().await.unwrap(), "ok");
    assert_eq!(server.resumed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_fresh_pool_labels_its_tickets_with_the_roots_it_loaded() {
    let server = resuming_server().await;
    let ca_file = scratch_file("fresh-pool-ca.pem", &server.pem);
    let original = http1_session(bare_trust().add_ca_file(&ca_file));
    std::fs::write(&ca_file, unrelated_pem()).unwrap();
    let copy = original.fresh_pool();
    let first = copy.get(server.url.clone()).send().await.unwrap();
    assert_eq!(first.text().await.unwrap(), "ok");
    let state = copy.state();

    let rotated = http1_session(bare_trust().add_ca_file(&ca_file));
    state.restore_into(&rotated);
    let refused = rotated.get(server.url).send().await.unwrap_err();
    assert!(refused.tls().is_some(), "{refused:?}");
    assert_eq!(server.resumed.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_saved_tls_session_does_not_resume_after_environment_roots_change() {
    let server = resuming_server().await;
    let trusted = scratch_file("env-trusted.pem", &server.pem);
    let other = scratch_file("env-other.pem", &unrelated_pem());
    let env_trust = || TlsTrustConfig::new().system_roots(false).env_roots(true);
    // SAFETY: no other test in this binary builds a session with environment roots, so no thread reads SSL_CERT_FILE while it changes.
    unsafe { std::env::set_var("SSL_CERT_FILE", &trusted) };
    let before = http1_session(env_trust());
    let first = before.get(server.url.clone()).send().await.unwrap();
    assert_eq!(first.text().await.unwrap(), "ok");
    let state = before.state();

    // SAFETY: see above.
    unsafe { std::env::set_var("SSL_CERT_FILE", &other) };
    let after = http1_session(env_trust());
    // SAFETY: see above.
    unsafe { std::env::remove_var("SSL_CERT_FILE") };
    state.restore_into(&after);
    let refused = after.get(server.url).send().await.unwrap_err();
    assert!(refused.tls().is_some(), "{refused:?}");
    assert_eq!(server.resumed.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_saved_tls_session_does_not_resume_after_the_client_certificate_rotates() {
    let server = resuming_server().await;
    let (cert, key) = tls_support::self_signed();
    let chain = scratch_file("client-chain.pem", &cert.to_pem().unwrap());
    let private = scratch_file("client-key.pem", &key.private_key_to_pem_pkcs8().unwrap());
    let with_identity = || {
        http1_session(
            bare_trust()
                .add_ca_der(server.der.clone())
                .client_identity(&chain, &private),
        )
    };
    let before = with_identity();
    let first = before.get(server.url.clone()).send().await.unwrap();
    assert_eq!(first.text().await.unwrap(), "ok");
    let state = before.state();

    let (next_cert, next_key) = tls_support::self_signed();
    std::fs::write(&chain, next_cert.to_pem().unwrap()).unwrap();
    std::fs::write(&private, next_key.private_key_to_pem_pkcs8().unwrap()).unwrap();
    let after = with_identity();
    state.restore_into(&after);
    let again = after.get(server.url.clone()).send().await.unwrap();
    assert_eq!(again.text().await.unwrap(), "ok");
    assert_eq!(server.resumed.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_client_certificate_in_a_trusted_certificate_block_loads() {
    let server = resuming_server().await;
    let (cert, key) = tls_support::self_signed();
    let pem = String::from_utf8(cert.to_pem().unwrap())
        .unwrap()
        .replace("CERTIFICATE-----", "TRUSTED CERTIFICATE-----");
    let chain = scratch_file("trusted-client-chain.pem", pem.as_bytes());
    let private = scratch_file(
        "trusted-client-key.pem",
        &key.private_key_to_pem_pkcs8().unwrap(),
    );
    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .tls_trust(
            bare_trust()
                .add_ca_der(server.der.clone())
                .client_identity(&chain, &private),
        )
        .build()
        .unwrap();
    let response = session.get(server.url).send().await.unwrap();
    assert_eq!(response.text().await.unwrap(), "ok");
}

#[tokio::test]
async fn the_server_receives_the_client_leaf_and_its_chain_in_order() {
    use leyline_bssl::ssl::{SslContextBuilder, SslMethod, SslVerifyMode};
    let (server_cert, server_key) = tls_support::self_signed();
    let server_der = server_cert.to_der().unwrap();
    let mut context = SslContextBuilder::new(SslMethod::tls()).unwrap();
    context.set_certificate(&server_cert).unwrap();
    context.set_private_key(&server_key).unwrap();
    context.set_verify_callback(SslVerifyMode::PEER, |_, _| true);
    let context = context.build();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let ssl = leyline_bssl::ssl::Ssl::new(&context).unwrap();
        let stream = leyline_bssl_tokio::SslStreamBuilder::new(ssl, tcp)
            .accept()
            .await
            .unwrap();
        let leaf = stream.ssl().peer_certificate().map(|c| c.to_der().unwrap());
        let chain: Vec<Vec<u8>> = stream
            .ssl()
            .peer_cert_chain()
            .map(|chain| chain.iter().map(|c| c.to_der().unwrap()).collect())
            .unwrap_or_default();
        seen_tx.send((leaf, chain)).unwrap();
        reply_ok(stream).await;
    });

    let (leaf, leaf_key) = tls_support::self_signed();
    let (intermediate, _) = tls_support::self_signed();
    let mut pem = String::from_utf8(leaf.to_pem().unwrap())
        .unwrap()
        .replace("CERTIFICATE-----", "TRUSTED CERTIFICATE-----");
    pem.push_str(&String::from_utf8(intermediate.to_pem().unwrap()).unwrap());
    let chain = scratch_file("ordered-client-chain.pem", pem.as_bytes());
    let private = scratch_file(
        "ordered-client-key.pem",
        &leaf_key.private_key_to_pem_pkcs8().unwrap(),
    );
    let session = http1_session(
        bare_trust()
            .add_ca_der(server_der)
            .client_identity(&chain, &private),
    );
    let response = session
        .get(format!("https://127.0.0.1:{port}/"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.text().await.unwrap(), "ok");
    let (seen_leaf, seen_chain) = seen_rx.await.unwrap();
    assert_eq!(seen_leaf, Some(leaf.to_der().unwrap()));
    let intermediate = intermediate.to_der().unwrap();
    let after_leaf: Vec<&Vec<u8>> = seen_chain
        .iter()
        .filter(|der| **der != leaf.to_der().unwrap())
        .collect();
    assert_eq!(after_leaf, [&intermediate]);
}

#[tokio::test]
async fn restored_tickets_for_other_trust_are_not_kept() {
    let server = resuming_server().await;
    let unpinned = http1_session(bare_trust().add_ca_der(server.der.clone()));
    let first = unpinned.get(server.url.clone()).send().await.unwrap();
    assert_eq!(first.text().await.unwrap(), "ok");
    let state = unpinned.state();
    let saved = serde_json::to_value(&state).unwrap();
    assert!(!saved["tls_sessions"].as_array().unwrap().is_empty());

    let pinned = http1_session(
        bare_trust()
            .add_ca_der(server.der)
            .add_pinned_leaf_sha256([7; 32]),
    );
    state.restore_into(&pinned);
    let kept = serde_json::to_value(pinned.state()).unwrap();
    assert_eq!(kept["tls_sessions"].as_array().map(Vec::len), Some(0));
}

async fn tunnel_proxy() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut client, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut buf = [0u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match client.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(read) => head.extend_from_slice(&buf[..read]),
                    }
                }
                let line = String::from_utf8_lossy(&head)
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                let Some(target) = line.split_whitespace().nth(1) else {
                    return;
                };
                let Ok(mut upstream) = TcpStream::connect(target).await else {
                    return;
                };
                if client
                    .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                    .await
                    .is_err()
                {
                    return;
                }
                drop(tokio::io::copy_bidirectional(&mut client, &mut upstream).await);
            });
        }
    });
    port
}

#[tokio::test]
async fn saved_tickets_through_a_proxy_hold_no_proxy_credentials() {
    let server = resuming_server().await;
    let proxy = tunnel_proxy().await;
    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .tls_trust(bare_trust().add_ca_der(server.der))
        .proxy(
            leyline::ProxyConfig::new()
                .env(false)
                .rule(leyline::ProxyRule::all(format!(
                    "http://tunneluser:tunnelsecret@127.0.0.1:{proxy}"
                ))),
        )
        .build()
        .unwrap();
    let response = session.get(server.url).send().await.unwrap();
    assert_eq!(response.text().await.unwrap(), "ok");
    let saved = serde_json::to_string(&session.state()).unwrap();
    assert!(saved.contains("tls_sessions"), "{saved}");
    assert!(
        !saved.contains("tunneluser"),
        "the proxy user name was saved"
    );
    assert!(
        !saved.contains("tunnelsecret"),
        "the proxy password was saved"
    );
}
