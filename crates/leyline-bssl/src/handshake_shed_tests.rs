use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use foreign_types::ForeignTypeRef;

use crate::asn1::Asn1Time;
use crate::ffi;
use crate::hash::MessageDigest;
use crate::pkey::PKey;
use crate::rsa::Rsa;
use crate::ssl::{Ssl, SslMethod, SslVerifyMode};
use crate::x509::{X509Name, X509};

fn identity() -> (X509, PKey<crate::pkey::Private>) {
    let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let mut name = X509Name::builder().unwrap();
    name.append_entry_by_text("CN", "localhost").unwrap();
    let name = name.build();
    let mut builder = X509::builder().unwrap();
    builder.set_version(2).unwrap();
    builder.set_subject_name(&name).unwrap();
    builder.set_issuer_name(&name).unwrap();
    builder.set_pubkey(&key).unwrap();
    let not_before = Asn1Time::days_from_now(0).unwrap();
    let not_after = Asn1Time::days_from_now(1).unwrap();
    builder.set_not_before(&not_before).unwrap();
    builder.set_not_after(&not_after).unwrap();
    builder.sign(&key, MessageDigest::sha256()).unwrap();
    (builder.build(), key)
}

fn clear_after_handshake(shed: bool) -> i32 {
    let (cert, key) = identity();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (sock, _) = listener.accept().unwrap();
        let mut ctx = crate::ssl::SslContext::builder(SslMethod::tls()).unwrap();
        ctx.set_certificate(&cert).unwrap();
        ctx.set_private_key(&key).unwrap();
        let ssl = Ssl::new(&ctx.build()).unwrap();
        let mut stream = ssl.accept(sock).unwrap();
        let mut buf = [0u8; 4];
        let _ = stream.read(&mut buf);
    });
    let sock = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut ctx = crate::ssl::SslContext::builder(SslMethod::tls()).unwrap();
    ctx.set_verify(SslVerifyMode::NONE);
    let mut ssl = Ssl::new(&ctx.build()).unwrap();
    if shed {
        ssl.shed_handshake_config();
    }
    let mut stream = ssl.connect(sock).unwrap();
    stream.write_all(b"ping").unwrap();
    let cleared = unsafe { ffi::SSL_clear(stream.ssl().as_ptr()) };
    drop(stream);
    server.join().unwrap();
    cleared
}

#[test]
fn shed_drops_config_after_the_handshake() {
    assert_eq!(clear_after_handshake(false), 1);
    assert_eq!(clear_after_handshake(true), 0);
}
