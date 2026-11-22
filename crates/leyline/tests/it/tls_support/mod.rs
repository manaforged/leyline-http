#![allow(
    dead_code,
    reason = "shared by several test binaries; each binary uses a subset"
)]

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use leyline_bssl::asn1::Asn1Time;
use leyline_bssl::bn::{BigNum, MsbOption};
use leyline_bssl::ec::{EcGroup, EcKey};
use leyline_bssl::hash::MessageDigest;
use leyline_bssl::nid::Nid;
use leyline_bssl::pkey::{PKey, Private};
use leyline_bssl::ssl::{AlpnError, Ssl, SslContextBuilder, SslMethod, select_next_proto};
use leyline_bssl::x509::extension::{BasicConstraints, SubjectAlternativeName};
use leyline_bssl::x509::{X509, X509NameBuilder};
use leyline_bssl_tokio::SslStream;
use tokio::net::{TcpListener, TcpStream};

pub fn self_signed() -> (X509, PKey<Private>) {
    self_signed_for(&["127.0.0.1"], true)
}

pub fn self_signed_dns(host: &str) -> (X509, PKey<Private>) {
    self_signed_for(&[host], false)
}

pub fn self_signed_dns_names(hosts: &[&str]) -> (X509, PKey<Private>) {
    self_signed_for(hosts, false)
}

fn self_signed_for(subjects: &[&str], is_ip: bool) -> (X509, PKey<Private>) {
    let key = PKey::from_ec_key(
        EcKey::generate(&EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap()).unwrap(),
    )
    .unwrap();
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_text("CN", subjects[0]).unwrap();
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
    let mut san = SubjectAlternativeName::new();
    for subject in subjects {
        if is_ip {
            san.ip(subject);
        } else {
            san.dns(subject);
        }
    }
    let san = san.build(&cert.x509v3_context(None, None)).unwrap();
    cert.append_extension(&san).unwrap();
    cert.sign(&key, MessageDigest::sha256()).unwrap();
    (cert.build(), key)
}

pub fn build_ech_config_list(
    config_id: u8,
    public_name: &str,
) -> (
    leyline_bssl::hpke::HpkeKey,
    Vec<u8>,
    leyline_bssl::ssl::SslEchKeys,
) {
    let key = leyline_bssl::hpke::HpkeKey::generate_x25519().unwrap();
    let single = leyline_bssl::ssl::marshal_ech_config(config_id, &key, public_name, 64).unwrap();
    let mut builder = leyline_bssl::ssl::SslEchKeys::builder().unwrap();
    builder.add(true, &single, &key).unwrap();
    let keys = builder.build();
    let list = keys.marshal_retry_configs().unwrap();
    (key, list, keys)
}

pub async fn serve_h1_ok(mut stream: SslStream<TcpStream>, body: &str) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut buf = [0u8; 2048];
    if stream.read(&mut buf).await.is_err() {
        return;
    }
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    drop(stream.write_all(resp.as_bytes()).await);
    drop(stream.shutdown().await);
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
