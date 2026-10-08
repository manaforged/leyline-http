#![allow(
    dead_code,
    reason = "shared by several test binaries; each binary uses a subset"
)]

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use leyline_bssl::asn1::Asn1Time;
use leyline_bssl::bn::{BigNum, MsbOption};
use leyline_bssl::hash::MessageDigest;
use leyline_bssl::pkey::{PKey, Private};
use leyline_bssl::rsa::Rsa;
use leyline_bssl::ssl::{AlpnError, Ssl, SslContextBuilder, SslMethod, select_next_proto};
use leyline_bssl::x509::extension::{BasicConstraints, SubjectAlternativeName};
use leyline_bssl::x509::{X509, X509NameBuilder};
use leyline_bssl_tokio::SslStream;
use tokio::net::{TcpListener, TcpStream};

pub fn self_signed() -> (X509, PKey<Private>) {
    let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_text("CN", "127.0.0.1").unwrap();
    let name = name.build();
    let mut cert = X509::builder().unwrap();
    cert.set_version(2).unwrap();
    let mut serial = BigNum::new().unwrap();
    serial.rand(159, MsbOption::MAYBE_ZERO, false).unwrap();
    cert.set_serial_number(&serial.to_asn1_integer().unwrap())
        .unwrap();
    cert.set_subject_name(&name).unwrap();
    cert.set_issuer_name(&name).unwrap();
    cert.set_pubkey(&key).unwrap();
    cert.set_not_before(&Asn1Time::days_from_now(0).unwrap())
        .unwrap();
    cert.set_not_after(&Asn1Time::days_from_now(1).unwrap())
        .unwrap();
    cert.append_extension(&BasicConstraints::new().critical().ca().build().unwrap())
        .unwrap();
    let san = SubjectAlternativeName::new()
        .ip("127.0.0.1")
        .build(&cert.x509v3_context(None, None))
        .unwrap();
    cert.append_extension(&san).unwrap();
    cert.sign(&key, MessageDigest::sha256()).unwrap();
    (cert.build(), key)
}

pub async fn tls_server<F, Fut>(
    cert: X509,
    key: PKey<Private>,
    connections: Arc<AtomicUsize>,
    serve: F,
) -> u16
where
    F: Fn(SslStream<TcpStream>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    tls_server_per_connection(
        cert,
        key,
        connections,
        |_| H2,
        move |_, stream| serve(stream),
    )
    .await
}

pub const H2: &[u8] = b"\x02h2";
pub const HTTP11: &[u8] = b"\x08http/1.1";

pub async fn tls_server_per_connection<A, F, Fut>(
    cert: X509,
    key: PKey<Private>,
    connections: Arc<AtomicUsize>,
    alpn: A,
    serve: F,
) -> u16
where
    A: Fn(usize) -> &'static [u8] + Send + Sync + 'static,
    F: Fn(usize, SslStream<TcpStream>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("local address").port();
    let accepted = Arc::new(AtomicUsize::new(0));
    let current = Arc::clone(&accepted);
    let mut context = SslContextBuilder::new(SslMethod::tls()).expect("TLS context");
    context.set_certificate(&cert).expect("certificate");
    context.set_private_key(&key).expect("private key");
    context.set_alpn_select_callback(move |_, offered| {
        select_next_proto(alpn(current.load(Ordering::SeqCst)), offered).ok_or(AlpnError::NOACK)
    });
    let context = context.build();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            connections.fetch_add(1, Ordering::SeqCst);
            let index = accepted.load(Ordering::SeqCst);
            let ssl = Ssl::new(&context).expect("TLS session");
            let handshake = leyline_bssl_tokio::SslStreamBuilder::new(ssl, tcp)
                .accept()
                .await;
            accepted.fetch_add(1, Ordering::SeqCst);
            if let Ok(stream) = handshake {
                tokio::spawn(serve(index, stream));
            }
        }
    });
    port
}
