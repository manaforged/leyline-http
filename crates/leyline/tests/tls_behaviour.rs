#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
#[path = "tls_support/mod.rs"]
mod tls_support;

use std::net::SocketAddr;
use std::sync::atomic::AtomicUsize;
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
    let _ = stream
        .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
        .await;
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

    assert!(fetch(bare_trust(), url.clone()).await.is_err());
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
    assert!(
        fetch(
            bare_trust().add_ca_der(der).add_pinned_leaf_sha256([7; 32]),
            url
        )
        .await
        .is_err()
    );
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
